use libtest_mimic::Failed;
use papo::Event;
use papo_conformance_macros::scenario;
use papo_core::Sasl;
use papo_proto::Message;
use papo_test_server::register_account;

use super::support::{EXPECT_TIMEOUT, PASSWORD, close_pair, join, joined_pair};
use crate::{Scenario, ScenarioResult};

/// delivers an authenticated sender account
#[scenario(requires(caps = ["account-tag", "account-notify"]))]
async fn account_notify(context: Scenario) -> ScenarioResult {
    let account = context.unique();
    register_account(&context.server, &account, PASSWORD).map_err(Failed::from)?;
    let sender_nick = context.unique();
    let receiver_nick = context.unique();
    let channel = format!("#{}", context.unique());
    let mut sender = context
        .client_with(&sender_nick, Some(Sasl::plain(&account, PASSWORD)))
        .await?;
    join(&context, &mut sender, &channel).await?;
    let mut receiver = context.client(&receiver_nick).await?;
    join(&context, &mut receiver, &channel).await?;
    let text = format!("authenticated message from {sender_nick}");
    sender
        .client
        .privmsg(&channel, &text)
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut receiver.events,
            |event| matches!(event, Event::Message(message) if message.raw_text() == text && message.account() == Some(account.as_str())),
            EXPECT_TIMEOUT,
        )
        .await?;
    close_pair(receiver, sender).await
}

/// notifies channel peers of away state
#[scenario(requires(caps = ["away-notify"]))]
async fn away_notify(context: Scenario) -> ScenarioResult {
    let (mut receiver, sender, _channel) = joined_pair(&context).await?;
    let away = "reviewing the IRCv3 specification";
    sender
        .client
        .send(Message::new("AWAY", [away]))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut receiver.events,
            |event| matches!(event, Event::AwayChanged { nick, message: Some(message) } if nick == &sender.welcome.nick && message == away),
            EXPECT_TIMEOUT,
        )
        .await?;
    close_pair(receiver, sender).await
}
