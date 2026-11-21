use super::{KeyScope, PeerProof, PeerThread};

const PREFIX: &str = "papo-peer:1;";

pub(super) fn render(thread: &PeerThread, proof: &PeerProof) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::from(PREFIX);
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
