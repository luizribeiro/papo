use super::{AccountPin, BouncerScope, InvalidPeerKey, PeerScope};

const PREFIX: &str = "papo-pin:1;";

pub(super) fn encode(
    scope: &PeerScope,
    casemapping: crate::Casemapping,
    account: &str,
) -> AccountPin {
    let mut pin = AccountPin {
        scope: scope.clone(),
        casemapping,
        account: account.to_owned(),
        id: None,
        encoded: String::new(),
    };
    pin.encoded = render(&pin);
    pin
}

pub(super) fn parse(value: &str) -> Result<AccountPin, InvalidPeerKey> {
    value.strip_prefix(PREFIX).ok_or(InvalidPeerKey)?;
    parse_fields(value)
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
        ["server", "casemap", "account"] | ["server", "casemap", "account", "id"] => {
            (PeerScope::server(&fields[0].1), 1)
        }
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

fn bouncer_scope(server: &str, login: &str, network: &str, upstream: &str) -> PeerScope {
    PeerScope {
        server: server.to_owned(),
        bouncer: Some(BouncerScope {
            login: login.to_owned(),
            network: network.to_owned(),
            upstream: upstream.to_owned(),
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

#[cfg(test)]
mod tests {
    #[test]
    fn percent_encoding_round_trips_utf8_and_delimiters() {
        let encoded = super::percent_encode("josé;net=id%");
        assert_eq!(encoded, "jos%C3%A9%3Bnet%3Did%25");
        assert_eq!(
            super::percent_decode(&encoded).as_deref(),
            Ok("josé;net=id%")
        );
    }

    #[test]
    fn malformed_percent_encoding_is_rejected() {
        for value in ["%", "%0", "%GG", "%FF"] {
            assert_eq!(super::percent_decode(value), Err(super::InvalidPeerKey));
        }
    }
}
