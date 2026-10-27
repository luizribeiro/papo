use crate::{Account, Error};

#[expect(dead_code, reason = "used by guarded direct construction")]
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

#[expect(dead_code, reason = "used by guarded direct construction")]
pub(crate) async fn release(client: &crate::Client, recipient: papo_core::RecipientGuard) {
    _ = client
        .commands
        .send(crate::driver::Command::DirectRelease { recipient })
        .await;
}
