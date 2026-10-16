use libtest_mimic::Failed;
use papo::Event;
use papo_conformance_macros::scenario;
use papo_proto::Message;

use super::support::{EXPECT_TIMEOUT, close_pair, join, joined_pair};
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

/// records every advertised membership prefix
#[scenario(requires(caps = ["multi-prefix"]))]
async fn multi_prefix(context: Scenario) -> ScenarioResult {
    let first_nick = context.unique();
    let second_nick = context.unique();
    let channel = format!("#{}", context.unique());
    let mut first = context.client(&first_nick).await?;
    join(&context, &mut first, &channel).await?;
    first
        .client
        .send(Message::new("MODE", [&channel, "+v", &first_nick]))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut first.events,
            |event| matches!(event, Event::ModeChanged { target, .. } if target == &channel),
            EXPECT_TIMEOUT,
        )
        .await?;
    let mut second = context.client(&second_nick).await?;
    join(&context, &mut second, &channel).await?;
    let prefixes = second.client.state(|state| {
        state
            .channel(&channel)
            .and_then(|channel| channel.member(&first_nick))
            .map(|member| member.prefixes().to_vec())
    });
    if prefixes.as_deref() != Some(&['@', '+']) {
        return Err(
            format!("NAMES did not preserve operator and voice prefixes: {prefixes:?}").into(),
        );
    }
    close_pair(first, second).await
}
