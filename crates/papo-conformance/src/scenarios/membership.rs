use libtest_mimic::Failed;
use papo_conformance_macros::scenario;

use super::support::{close_pair, joined_pair};
use crate::{Scenario, ScenarioResult};

/// records real names from extended JOIN
#[scenario(requires(caps = ["extended-join"]))]
async fn extended_join(context: Scenario) -> ScenarioResult {
    let (receiver, sender, _channel) = joined_pair(&context).await?;
    let observed = receiver.client.state(|state| {
        state
            .user(&sender.welcome.nick)
            .and_then(papo::User::realname)
            == Some("papo conformance client")
    });
    if !observed {
        return Err("extended JOIN did not populate the sender real name".into());
    }
    close_pair(receiver, sender).await
}

/// records usernames and hosts from NAMES
#[scenario(requires(caps = ["userhost-in-names"]))]
async fn userhost_in_names(context: Scenario) -> ScenarioResult {
    let (receiver, sender, _channel) = joined_pair(&context).await?;
    let userhost = receiver.client.state(|state| {
        state.user(&sender.welcome.nick).map(|user| {
            (
                user.username().map(str::to_owned),
                user.host().map(str::to_owned),
            )
        })
    });
    if !matches!(userhost, Some((Some(_), Some(_)))) {
        return Err(format!("NAMES did not include a complete userhost: {userhost:?}").into());
    }
    close_pair(receiver, sender).await
}

/// parses an extended WHO reply
#[scenario(requires(isupport = ["WHOX"]))]
async fn whox(context: Scenario) -> ScenarioResult {
    let nick = context.unique();
    let connection = context.client(&nick).await?;
    let entries = connection
        .client
        .who(&connection.welcome.nick)
        .await
        .map_err(Failed::from)?;
    let complete = entries.iter().any(|entry| {
        entry.nick == connection.welcome.nick
            && !entry.user.is_empty()
            && !entry.host.is_empty()
            && !entry.realname.is_empty()
    });
    if !complete {
        return Err("WHOX reply did not contain the registered user's identity".into());
    }
    connection.close().await
}
