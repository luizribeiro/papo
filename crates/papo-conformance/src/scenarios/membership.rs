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
    let observed = receiver
        .client
        .state()
        .user(&sender.welcome.nick)
        .and_then(papo::User::realname)
        == Some("papo conformance client");
    if !observed {
        return Err("extended JOIN did not populate the sender real name".into());
    }
    close_pair(receiver, sender).await
}

/// records usernames and hosts from NAMES
#[scenario(requires(caps = ["userhost-in-names"]))]
async fn userhost_in_names(context: Scenario) -> ScenarioResult {
    let (receiver, sender, _channel) = joined_pair(&context).await?;
    let userhost = receiver
        .client
        .state()
        .user(&sender.welcome.nick)
        .map(|user| {
            (
                user.username().map(str::to_owned),
                user.host().map(str::to_owned),
            )
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
    let casemapping = connection.client.state().casemapping();
    let complete = entries.iter().any(|entry| {
        casemapping.eq(&entry.nick, &connection.welcome.nick)
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
    let casemapping = first.client.state().casemapping();
    context
        .expect(
            &mut first.events,
            |event| {
                matches!(event, Event::ModeChanged(changed) if casemapping.eq(&changed.target, &channel))
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    let mut second = context.client(&second_nick).await?;
    join(&context, &mut second, &channel).await?;
    let prefixes = second
        .client
        .state()
        .channel(&channel)
        .and_then(|channel| channel.member(&first_nick))
        .map(|member| member.prefixes().to_vec());
    if prefixes.as_deref() != Some(&['@', '+']) {
        return Err(
            format!("NAMES did not preserve operator and voice prefixes: {prefixes:?}").into(),
        );
    }
    close_pair(first, second).await
}

/// notifies channel members about invitations
#[scenario(requires(caps = ["invite-notify"]))]
async fn invite_notify(context: Scenario) -> ScenarioResult {
    let (inviter, mut observer, channel) = joined_pair(&context).await?;
    let observer_nick = observer.welcome.nick.clone();
    if !observer.client.state().has_cap("invite-notify") {
        return Err("invite-notify observer did not negotiate the capability".into());
    }
    inviter
        .client
        .send(Message::new("MODE", [&channel, "+o", &observer_nick]))
        .await
        .map_err(Failed::from)?;
    let casemapping = observer.client.state().casemapping();
    context
        .expect(
            &mut observer.events,
            |event| {
                matches!(event, Event::ModeChanged(changed) if casemapping.eq(&changed.target, &channel))
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    let observer_is_op = observer
        .client
        .state()
        .channel(&channel)
        .and_then(|channel| channel.member(&observer_nick))
        .is_some_and(papo_core::Member::is_op);
    if !observer_is_op {
        return Err("invite-notify observer is not a channel operator".into());
    }
    let target_nick = context.unique();
    let target = context.client(&target_nick).await?;
    inviter
        .client
        .send(Message::new("INVITE", [&target_nick, &channel]))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut observer.events,
            |event| {
                matches!(event, Event::Raw(message) if {
                    message.command().eq_ignore_ascii_case("INVITE")
                        && message.params().first().is_some_and(|value| value == &target_nick)
                        && message.params().get(1).is_some_and(|value| value == &channel)
                })
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    observer.close().await?;
    close_pair(inviter, target).await
}
