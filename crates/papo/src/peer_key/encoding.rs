use super::{AccountPin, BouncerScope, InvalidPeerKey, PeerScope};

const PREFIX: &str = "papo-peer-v1:";

pub(super) fn encode(
    scope: &PeerScope,
    casemapping: crate::Casemapping,
    account: &str,
) -> AccountPin {
    let scope = legacy_scope(scope);
    let encoded = format!(
        "{PREFIX}{}:{}:{scope}{account}",
        casemapping.as_str(),
        scope.len()
    );
    AccountPin {
        scope,
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
    let account = casemapping.account(account);
    account.check().map_err(|_| InvalidPeerKey)?;
    Ok(AccountPin {
        scope: scope.to_owned(),
        casemapping,
        account: account.key().to_owned(),
        id: None,
        encoded: value.to_owned(),
    })
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
