use super::{AccountPin, BouncerScope, InvalidPeerKey, PeerScope};

const PREFIX: &str = "papo-peer-v1:";

pub(super) fn encode(
    scope: &PeerScope,
    casemapping: crate::Casemapping,
    account: &str,
) -> AccountPin {
    let encoded_scope = legacy_scope(scope);
    let encoded = format!(
        "{PREFIX}{}:{}:{encoded_scope}{account}",
        casemapping.as_str(),
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
    let casemapping = casemapping
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
