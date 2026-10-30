use std::{
    fs,
    io::{self, Read},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use papo::{Client, Endpoint, NetworkId};
use papo_core::sts::{StsPolicy, StsStore};

use crate::Scenario;

#[derive(Clone, Default)]
struct RecordingStsStore(Arc<Mutex<Option<StsPolicy>>>);

impl StsStore for RecordingStsStore {
    fn get(&self, host: &str) -> Option<StsPolicy> {
        self.0
            .lock()
            .unwrap()
            .as_ref()
            .filter(|policy| policy.host == host)
            .cloned()
    }

    fn put(&self, policy: StsPolicy) {
        *self.0.lock().unwrap() = Some(policy);
    }

    fn remove(&self, host: &str) {
        let mut policy = self.0.lock().unwrap();
        if policy.as_ref().is_some_and(|policy| policy.host == host) {
            *policy = None;
        }
    }
}

fn network_id_matches(candidate: &NetworkId, expected: &NetworkId) -> bool {
    candidate == expected
}

pub(super) async fn bouncer_networks(s: Scenario) -> papo_testing::Result<()> {
    let Some(bouncer) = s.server.bouncer() else {
        return s.skip("the selected server is not a managed bouncer");
    };
    let network_nick = papo_test_server::unique("alice");
    let login = bouncer.provision_network(&network_nick, "papo bouncer control", None)?;
    let address = s.server.plaintext_addr();
    let alice = s
        .connect_with(
            Client::builder(
                Endpoint::plaintext(address.ip().to_string(), address.port()).unwrap(),
                network_nick,
            )
            .username(login.username())
            .realname("papo bouncer control")
            .password(papo::Secret::new(login.password()))
            .allow_insecure_auth(true)
            .enable(papo::Feature::Bouncer),
        )
        .await?;
    let initial = alice.bouncer().networks().await?;
    assert_eq!(
        alice
            .state()
            .bouncer_networks()
            .cloned()
            .collect::<Vec<_>>(),
        initial
    );

    let upstream = bouncer.upstream_addr();
    let host = upstream.ip().to_string();
    let name = papo_test_server::unique("managed-network");
    let endpoint = format!("irc://{upstream}");
    let upstream_nick = alice
        .state()
        .casemapping()
        .nick(papo_test_server::unique("upstream-alice"));
    let id = alice
        .bouncer()
        .add(endpoint)
        .name(name.clone())
        .nick(upstream_nick)
        .await?;
    let added = alice.bouncer().networks().await?;
    let network = added
        .iter()
        .find(|network| network_id_matches(&network.id, &id))
        .expect("the added network is listed");
    assert_eq!(network.id, id);
    assert_eq!(network.name.as_deref(), Some(name.as_str()));
    assert_eq!(network.host.as_deref(), Some(host.as_str()));
    assert_eq!(network.port, Some(upstream.port()));

    let changed_name = papo_test_server::unique("renamed-network");
    let upstream_nick = alice
        .state()
        .casemapping()
        .nick(papo_test_server::unique("upstream-bob"));
    alice
        .bouncer()
        .change(&id)
        .name(changed_name.clone())
        .nick(upstream_nick)
        .await?;
    let changed = alice.bouncer().networks().await?;
    assert_eq!(
        changed
            .iter()
            .find(|network| network.id == id)
            .and_then(|network| network.name.as_deref()),
        Some(changed_name.as_str())
    );

    alice.bouncer().remove(&id).await?;
    let removed = alice.bouncer().networks().await?;
    assert!(
        removed.iter().all(|network| network.id != id),
        "the removed network was still listed"
    );
    Ok(())
}

pub(super) async fn sts(s: Scenario) -> papo_testing::Result<()> {
    let server = papo_test_server::TestServer::start_with_sts()?;
    let store = RecordingStsStore::default();
    let nick = papo_test_server::unique("alice-sts");
    let alice = s
        .connect_with(
            Client::builder(
                Endpoint::plaintext("localhost", server.plaintext_addr().port()).unwrap(),
                &nick,
            )
            .tls_ca_pem(fs::read(server.ca_pem_path())?)
            .sts_store(store.clone()),
        )
        .await?;
    assert!(alice.state().is_connected());
    assert_eq!(alice.state().nick(), nick.as_str());
    let policy = store
        .0
        .lock()
        .unwrap()
        .clone()
        .expect("the secure connection persisted STS");
    assert_eq!(policy.host, "localhost");
    assert_eq!(policy.port, server.tls_addr().port());
    assert_eq!(policy.duration, Duration::from_secs(3600));
    assert!(policy.is_active_at(std::time::SystemTime::now()));
    s.quit_all().await?;
    Ok(())
}

pub(super) async fn tls_sni(_s: Scenario) -> papo_testing::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let observer = thread::spawn(move || -> io::Result<String> {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        let hello = read_client_hello(&mut stream)?;
        parse_server_name(&hello)
    });

    let expected = "sni.papo.test";
    let endpoint = Endpoint::tls(address.ip().to_string(), address.port()).unwrap();
    let result = Client::builder(endpoint, papo_test_server::unique("sni-client"))
        .tls_server_name(expected)
        .connect()
        .await;
    assert!(result.is_err(), "the observer is not a TLS server");
    let server_name = observer
        .join()
        .map_err(|_| io::Error::other("ClientHello observer panicked"))??;
    assert_eq!(server_name, expected);
    Ok(())
}

fn read_client_hello(stream: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut handshake = Vec::new();
    loop {
        let mut header = [0_u8; 5];
        stream.read_exact(&mut header)?;
        if header[0] != 22 {
            return Err(io::Error::other(
                "first TLS handshake used a non-handshake record",
            ));
        }
        let length = usize::from(u16::from_be_bytes([header[3], header[4]]));
        let start = handshake.len();
        handshake.resize(start + length, 0);
        stream.read_exact(&mut handshake[start..])?;
        if handshake.len() >= 4 {
            let length = (usize::from(handshake[1]) << 16)
                | (usize::from(handshake[2]) << 8)
                | usize::from(handshake[3]);
            if handshake.len() >= length + 4 {
                handshake.truncate(length + 4);
                return Ok(handshake);
            }
        }
    }
}

fn parse_server_name(handshake: &[u8]) -> io::Result<String> {
    let mut input = handshake;
    if take(&mut input, 1)? != [1] {
        return Err(io::Error::other("TLS handshake was not a ClientHello"));
    }
    take(&mut input, 3)?;
    take(&mut input, 2 + 32)?;
    let session_id = usize::from(take(&mut input, 1)?[0]);
    take(&mut input, session_id)?;
    let cipher_suites = usize::from(take_u16(&mut input)?);
    take(&mut input, cipher_suites)?;
    let compression = usize::from(take(&mut input, 1)?[0]);
    take(&mut input, compression)?;
    let extensions = usize::from(take_u16(&mut input)?);
    let mut extensions = take(&mut input, extensions)?;
    while !extensions.is_empty() {
        let kind = take_u16(&mut extensions)?;
        let length = usize::from(take_u16(&mut extensions)?);
        let mut extension = take(&mut extensions, length)?;
        if kind != 0 {
            continue;
        }
        let names = usize::from(take_u16(&mut extension)?);
        let mut names = take(&mut extension, names)?;
        while !names.is_empty() {
            let kind = take(&mut names, 1)?[0];
            let length = usize::from(take_u16(&mut names)?);
            let name = take(&mut names, length)?;
            if kind == 0 {
                return std::str::from_utf8(name)
                    .map(str::to_owned)
                    .map_err(|_| io::Error::other("SNI name was not UTF-8"));
            }
        }
    }
    Err(io::Error::other("ClientHello omitted server_name"))
}

fn take<'a>(input: &mut &'a [u8], length: usize) -> io::Result<&'a [u8]> {
    if input.len() < length {
        return Err(io::Error::other("truncated ClientHello"));
    }
    let (value, remaining) = input.split_at(length);
    *input = remaining;
    Ok(value)
}

fn take_u16(input: &mut &[u8]) -> io::Result<u16> {
    let value = take(input, 2)?;
    Ok(u16::from_be_bytes([value[0], value[1]]))
}

pub(super) async fn server_password_waits_for_live_sts(_s: Scenario) -> papo_testing::Result<()> {
    let server = papo_test_server::TestServer::start_with_sts_and_hostname_lookup()?;
    let proxy = server.hostname_notice_proxy()?;
    let endpoint = format!("irc://localhost:{}", proxy.address().port());
    let result = Client::builder(endpoint, papo_test_server::unique("alice-pass-sts"))
        .password(papo::Secret::new("server-password-must-stay-encrypted"))
        .tls_ca_pem(fs::read(server.ca_pem_path())?)
        .connect()
        .await;

    let from_server = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let (_, bytes) = proxy.wire_bytes();
            if bytes
                .windows(b" CAP * LS ".len())
                .any(|line| line == b" CAP * LS ")
            {
                break bytes;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("Ergo answered CAP LS through the plaintext proxy");
    let (from_client, _) = proxy.wire_bytes();
    let from_client = String::from_utf8(from_client)?;
    assert!(from_client.contains("CAP LS 302\r\n"));
    assert!(
        !from_client.contains("PASS"),
        "plaintext wire contained PASS"
    );

    let from_server = String::from_utf8(from_server)?;
    let notice = from_server
        .find("NOTICE * :*** Looking up your hostname...")
        .unwrap_or_else(|| panic!("Ergo sent its hostname lookup notice: {from_server:?}"));
    let cap_ls = from_server
        .find(" CAP * LS ")
        .expect("Ergo answered CAP LS");
    assert!(
        notice < cap_ls,
        "Ergo's hostname notice did not precede CAP LS: {from_server:?}"
    );
    let (client, _events) = result.unwrap_or_else(|error| {
        panic!("STS reconnect failed: {error}; client={from_client:?}; server={from_server:?}")
    });
    client.quit("live STS credential check complete").await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use papo::NetworkId;
    use papo_core::sts::{StsPolicy, StsStore};

    use super::{RecordingStsStore, network_id_matches};

    #[test]
    fn recording_store_matches_and_removes_only_the_requested_host() {
        let policy = StsPolicy::new(
            "irc.example.net",
            6697,
            SystemTime::UNIX_EPOCH + Duration::from_secs(7200),
            Duration::from_secs(3600),
        );
        let store = RecordingStsStore::default();
        store.put(policy.clone());
        assert_eq!(store.get("irc.example.net"), Some(policy.clone()));
        assert_eq!(store.get("chat.example.net"), None);

        store.remove("chat.example.net");
        assert_eq!(store.get("irc.example.net"), Some(policy));
        store.remove("irc.example.net");
        assert_eq!(store.get("irc.example.net"), None);
    }

    #[test]
    fn network_ids_match_exactly() {
        let expected = NetworkId::from("irc.example.net");
        assert!(network_id_matches(&expected, &expected));
        assert!(!network_id_matches(
            &NetworkId::from("chat.example.net"),
            &expected
        ));
    }
}
