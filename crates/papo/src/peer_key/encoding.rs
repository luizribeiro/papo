use super::{
    AccountPin, AccountProof, BouncerScope, InvalidPeerKey, KeyScope, PeerProof, PeerScope,
    PeerThread,
};

const PREFIX: &str = "papo-pin:1;";

#[cfg(test)]
mod tests;

pub(super) fn encode(
    scope: &PeerScope,
    casemapping: crate::Casemapping,
    account: &str,
    id: Option<&str>,
) -> AccountPin {
    let mut pin = AccountPin {
        scope: scope.clone(),
        casemapping,
        account: account.to_owned(),
        id: id.map(str::to_owned),
        encoded: String::new(),
    };
    pin.encoded = render(&pin);
    pin
}

pub(super) fn parse(value: &str) -> Result<AccountPin, InvalidPeerKey> {
    value.strip_prefix(PREFIX).ok_or(InvalidPeerKey)?;
    parse_fields(value)
}

#[expect(dead_code, reason = "used when peer keys switch formats")]
pub(super) fn parse_peer(value: &str) -> Result<(PeerThread, PeerProof), InvalidPeerKey> {
    let value = value.strip_prefix("papo-peer:1;").ok_or(InvalidPeerKey)?;
    let fields = value
        .split(';')
        .map(|field| {
            let (name, value) = field.split_once('=').ok_or(InvalidPeerKey)?;
            Ok((name, percent_decode(value)?))
        })
        .collect::<Result<Vec<_>, InvalidPeerKey>>()?;
    let names = fields.iter().map(|field| field.0).collect::<Vec<_>>();
    let (scope, offset) = parse_scope(&fields, &names)?;
    let remaining = &names[offset..];
    let proof = match remaining {
        ["nick-casemap", "nick", "proof"] if fields[offset + 2].1 == "unverified" => {
            PeerProof::Unverified(None)
        }
        [
            "nick-casemap",
            "nick",
            "proof",
            "account-casemap",
            "account",
        ] if fields[offset + 2].1 == "account" => {
            PeerProof::Account(Box::new(parse_account(&fields, offset + 3, false)?), None)
        }
        [
            "nick-casemap",
            "nick",
            "proof",
            "account-casemap",
            "account",
            "id",
        ] if fields[offset + 2].1 == "account" => {
            PeerProof::Account(Box::new(parse_account(&fields, offset + 3, true)?), None)
        }
        _ => return Err(InvalidPeerKey),
    };
    if matches!(scope, KeyScope::Connection(_)) && matches!(proof, PeerProof::Account(_, _)) {
        return Err(InvalidPeerKey);
    }
    let casemapping = fields[offset]
        .1
        .parse::<crate::Casemapping>()
        .map_err(|_| InvalidPeerKey)?;
    let nick = casemapping.nick(&fields[offset + 1].1);
    if nick.key().is_empty() {
        return Err(InvalidPeerKey);
    }
    Ok((
        PeerThread {
            scope,
            casemapping,
            nick: nick.key().to_owned(),
        },
        proof,
    ))
}

fn parse_fields(value: &str) -> Result<AccountPin, InvalidPeerKey> {
    let fields = value[PREFIX.len()..]
        .split(';')
        .map(|field| {
            let (name, value) = field.split_once('=').ok_or(InvalidPeerKey)?;
            Ok((name, percent_decode(value)?))
        })
        .collect::<Result<Vec<_>, InvalidPeerKey>>()?;
    let names = fields.iter().map(|field| field.0).collect::<Vec<_>>();
    let (scope, account_offset) = match names.as_slice() {
        ["server", "casemap", "account"] | ["server", "casemap", "account", "id"] => (
            PeerScope::server(&crate::sts::normalize_host(&fields[0].1)),
            1,
        ),
        [
            "server",
            "bouncer-login",
            "netid",
            "upstream",
            "casemap",
            "account",
        ]
        | [
            "server",
            "bouncer-login",
            "netid",
            "upstream",
            "casemap",
            "account",
            "id",
        ] => (
            bouncer_scope(&fields[0].1, &fields[1].1, &fields[2].1, &fields[3].1),
            4,
        ),
        _ => return Err(InvalidPeerKey),
    };
    let casemap = &fields[account_offset].1;
    let account = &fields[account_offset + 1].1;
    let id = fields.get(account_offset + 2).map(|field| &field.1);
    if scope.server.is_empty()
        || scope.bouncer.as_ref().is_some_and(|scope| {
            scope.login.is_empty() || scope.network.is_empty() || scope.upstream.is_empty()
        })
        || account.is_empty()
        || id.is_some_and(String::is_empty)
    {
        return Err(InvalidPeerKey);
    }
    let casemapping = casemap
        .parse::<crate::Casemapping>()
        .map_err(|_| InvalidPeerKey)?;
    let account = casemapping.account(account);
    account.check().map_err(|_| InvalidPeerKey)?;
    let mut pin = AccountPin {
        scope,
        casemapping,
        account: account.key().to_owned(),
        id: id.cloned(),
        encoded: String::new(),
    };
    pin.encoded = render(&pin);
    Ok(pin)
}

#[expect(dead_code, reason = "used by the peer-key parser")]
fn parse_scope(
    fields: &[(&str, String)],
    names: &[&str],
) -> Result<(KeyScope, usize), InvalidPeerKey> {
    if names.first() == Some(&"connection") {
        let connection = &fields[0].1;
        return (!connection.is_empty())
            .then(|| (KeyScope::Connection(connection.clone()), 1))
            .ok_or(InvalidPeerKey);
    }
    let bouncer =
        names.get(..4) == Some(["server", "bouncer-login", "netid", "upstream"].as_slice());
    let offset = if bouncer { 4 } else { 1 };
    if names.first() != Some(&"server") || names.len() <= offset {
        return Err(InvalidPeerKey);
    }
    let server = crate::sts::normalize_host(&fields[0].1);
    let bouncer = bouncer.then(|| BouncerScope {
        login: fields[1].1.clone(),
        network: fields[2].1.clone(),
        upstream: crate::sts::normalize_host(&fields[3].1),
    });
    if server.is_empty()
        || bouncer.as_ref().is_some_and(|scope| {
            scope.login.is_empty() || scope.network.is_empty() || scope.upstream.is_empty()
        })
    {
        return Err(InvalidPeerKey);
    }
    Ok((KeyScope::Network(PeerScope { server, bouncer }), offset))
}

#[expect(dead_code, reason = "used by the peer-key parser")]
fn parse_account(
    fields: &[(&str, String)],
    offset: usize,
    has_id: bool,
) -> Result<AccountProof, InvalidPeerKey> {
    let casemapping = fields[offset]
        .1
        .parse::<crate::Casemapping>()
        .map_err(|_| InvalidPeerKey)?;
    let account = casemapping.account(&fields[offset + 1].1);
    account.check().map_err(|_| InvalidPeerKey)?;
    let id = has_id.then(|| fields[offset + 2].1.clone());
    if id.as_ref().is_some_and(String::is_empty) {
        return Err(InvalidPeerKey);
    }
    Ok(AccountProof {
        casemapping,
        account: account.key().to_owned(),
        id,
    })
}

fn render(pin: &AccountPin) -> String {
    use std::fmt::Write as _;
    let mut encoded = format!("{PREFIX}server={}", percent_encode(&pin.scope.server));
    if let Some(bouncer) = &pin.scope.bouncer {
        write!(
            encoded,
            ";bouncer-login={};netid={};upstream={}",
            percent_encode(&bouncer.login),
            percent_encode(&bouncer.network),
            percent_encode(&bouncer.upstream)
        )
        .expect("writing to a string cannot fail");
    }
    write!(
        encoded,
        ";casemap={};account={}",
        pin.casemapping.as_str(),
        percent_encode(&pin.account)
    )
    .expect("writing to a string cannot fail");
    if let Some(id) = &pin.id {
        write!(encoded, ";id={}", percent_encode(id)).expect("writing to a string cannot fail");
    }
    encoded
}

#[expect(dead_code, reason = "used when peer keys switch formats")]
pub(super) fn render_peer(thread: &PeerThread, proof: &PeerProof) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::from("papo-peer:1;");
    match &thread.scope {
        KeyScope::Network(scope) => {
            write!(encoded, "server={}", percent_encode(&scope.server))
                .expect("writing to a string cannot fail");
            if let Some(bouncer) = &scope.bouncer {
                write!(
                    encoded,
                    ";bouncer-login={};netid={};upstream={}",
                    percent_encode(&bouncer.login),
                    percent_encode(&bouncer.network),
                    percent_encode(&bouncer.upstream)
                )
                .expect("writing to a string cannot fail");
            }
        }
        KeyScope::Connection(connection) => {
            write!(encoded, "connection={}", percent_encode(connection))
                .expect("writing to a string cannot fail");
        }
    }
    write!(
        encoded,
        ";nick-casemap={};nick={}",
        thread.casemapping.as_str(),
        percent_encode(&thread.nick)
    )
    .expect("writing to a string cannot fail");
    match proof {
        PeerProof::Unverified(_) => encoded.push_str(";proof=unverified"),
        PeerProof::Account(account, _) => {
            write!(
                encoded,
                ";proof=account;account-casemap={};account={}",
                account.casemapping.as_str(),
                percent_encode(&account.account)
            )
            .expect("writing to a string cannot fail");
            if let Some(id) = &account.id {
                write!(encoded, ";id={}", percent_encode(id))
                    .expect("writing to a string cannot fail");
            }
        }
    }
    encoded
}

fn bouncer_scope(server: &str, login: &str, network: &str, upstream: &str) -> PeerScope {
    PeerScope {
        server: crate::sts::normalize_host(server),
        bouncer: Some(BouncerScope {
            login: login.to_owned(),
            network: network.to_owned(),
            upstream: crate::sts::normalize_host(upstream),
        }),
    }
}

fn percent_encode(value: &str) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            write!(encoded, "%{byte:02X}").expect("writing to a string cannot fail");
        }
    }
    encoded
}

fn percent_decode(value: &str) -> Result<String, InvalidPeerKey> {
    let mut decoded = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte != b'%' {
            decoded.push(byte);
            continue;
        }
        let high = bytes.next().and_then(hex).ok_or(InvalidPeerKey)?;
        let low = bytes.next().and_then(hex).ok_or(InvalidPeerKey)?;
        decoded.push(high * 16 + low);
    }
    String::from_utf8(decoded).map_err(|_| InvalidPeerKey)
}

const fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
