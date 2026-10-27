use crate::{Account, Error};

pub(crate) async fn command(
    client: &crate::Client,
    nick: String,
    account: Option<Account>,
    acquire: bool,
) -> Result<papo_core::RecipientGuard, Error> {
    let (result, receiver) = tokio::sync::oneshot::channel();
    let command = if acquire {
        crate::driver::Command::DirectAcquire {
            nick,
            account,
            result,
        }
    } else {
        crate::driver::Command::DirectRefresh {
            nick,
            account: account.unwrap_or(Account::Unknown),
            result,
        }
    };
    client
        .commands
        .send(command)
        .await
        .map_err(|_| Error::connection_lost())?;
    receiver.await.map_err(|_| Error::connection_lost())?
}

pub(crate) async fn release(client: &crate::Client, recipient: papo_core::RecipientGuard) {
    _ = client
        .commands
        .send(crate::driver::Command::DirectRelease { recipient })
        .await;
}

pub(crate) fn require_monitor_slot(state: &crate::State, nick: &str) -> Result<(), Error> {
    crate::Client::require_feature(state, crate::Feature::Monitor)?;
    if state.monitored().any(|known| known == nick) {
        return Ok(());
    }
    if let Some(crate::isupport::LimitValue::Limited(limit)) = state.isupport().monitor()
        && state.monitored().len() >= limit
    {
        return Err(Error::MonitorFull { limit });
    }
    Ok(())
}

pub(crate) fn who_account(
    client: &crate::Client,
    nick: &str,
    response: &papo_core::Response,
) -> Result<Account, Error> {
    if let Some(error) = response.error() {
        return Err(Error::server(error));
    }
    let account = response.lines.iter().find_map(|line| {
        let line = line.as_ref();
        (line.command_is("354") && line.param(1).is_some_and(|found| found == nick))
            .then(|| line.param(2))
            .flatten()
    });
    match account {
        None => Err(Error::TargetGone),
        Some("0" | "*") => Ok(Account::LoggedOut),
        Some(account) => Ok(Account::LoggedIn(
            client.state().name_rules().account(account),
        )),
    }
}
