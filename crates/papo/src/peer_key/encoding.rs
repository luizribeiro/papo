use super::{AccountPin, BouncerScope, InvalidPeerKey, PeerScope};

const PREFIX: &str = "papo-peer-v1:";

pub(super) fn encode(
    scope: &PeerScope,
    casemapping: crate::Casemapping,
    account: &str,
) -> AccountPin {
    let encoded_scope = legacy_scope(scope);
    let encoded_casemap = percent_encode(casemapping.as_str());
    let encoded = format!(
        "{PREFIX}{encoded_casemap}:{}:{encoded_scope}{account}",
        encoded_scope.len()
    );
    AccountPin {
        scope: scope.clone(),
        casemapping,
        account: account.to_owned(),
        id: None,
        encoded,
    }
}

pub(super) fn parse(value: &str) -> Result<AccountPin, InvalidPeerKey> {
    let (casemapping, rest) = value
        .strip_prefix(PREFIX)
        .and_then(|value| value.split_once(':'))
        .ok_or(InvalidPeerKey)?;
    let casemapping = percent_decode(casemapping)?
        .parse::<crate::Casemapping>()
        .map_err(|_| InvalidPeerKey)?;
    let (scope_len, joined) = rest.split_once(':').ok_or(InvalidPeerKey)?;
    let scope_len = scope_len.parse::<usize>().map_err(|_| InvalidPeerKey)?;
    let (scope, account) = joined
        .get(..scope_len)
        .zip(joined.get(scope_len..))
        .ok_or(InvalidPeerKey)?;
    if scope.is_empty() || account.is_empty() {
        return Err(InvalidPeerKey);
    }
    let parsed_scope = parse_legacy_scope(scope).ok_or(InvalidPeerKey)?;
    let account = casemapping.account(account);
    account.check().map_err(|_| InvalidPeerKey)?;
    Ok(AccountPin {
        scope: parsed_scope,
        casemapping,
        account: account.key().to_owned(),
        id: None,
        encoded: value.to_owned(),
    })
}

fn parse_legacy_scope(value: &str) -> Option<PeerScope> {
    let mut rest = value.strip_prefix("server=")?;
    let server = take_legacy_value(&mut rest)?;
    if rest.is_empty() {
        return Some(PeerScope::server(&server));
    }
    rest = rest.strip_prefix(";login=")?;
    let login = take_legacy_value(&mut rest)?;
    rest = rest.strip_prefix(";network=")?;
    let network = take_legacy_value(&mut rest)?;
    rest = rest.strip_prefix(";upstream=")?;
    let upstream = take_legacy_value(&mut rest)?;
    rest.is_empty().then_some(PeerScope {
        server,
        bouncer: Some(BouncerScope {
            login,
            network,
            upstream,
        }),
    })
}

fn take_legacy_value(rest: &mut &str) -> Option<String> {
    let (len, value) = rest.split_once(':')?;
    let len = len.parse::<usize>().ok()?;
    let (value, remaining) = value.get(..len).zip(value.get(len..))?;
    *rest = remaining;
    Some(value.to_owned())
}

fn legacy_scope(scope: &PeerScope) -> String {
    let mut value = format!("server={}:{}", scope.server.len(), scope.server);
    if let Some(BouncerScope {
        login,
        network,
        upstream,
    }) = &scope.bouncer
    {
        use std::fmt::Write as _;
        write!(
            value,
            ";login={}:{};network={}:{};upstream={}:{}",
            login.len(),
            login,
            network.len(),
            network,
            upstream.len(),
            upstream
        )
        .expect("writing to a string cannot fail");
    }
    value
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
