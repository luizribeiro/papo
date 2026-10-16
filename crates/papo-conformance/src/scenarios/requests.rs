use libtest_mimic::Failed;
use papo::{Event, event::StandardReplyKind};
use papo_conformance_macros::scenario;
use papo_proto::Message;

use super::support::{EXPECT_TIMEOUT, history_page};
use crate::{Scenario, ScenarioResult};

/// correlates a labelled WHOIS response
#[scenario(requires(caps = ["labeled-response"]))]
async fn labeled_response(context: Scenario) -> ScenarioResult {
    let nick = context.unique();
    let connection = context.client(&nick).await?;
    let whois = connection
        .client
        .whois(&connection.welcome.nick)
        .await
        .map_err(Failed::from)?;
    if whois.nick != connection.welcome.nick {
        return Err("labelled WHOIS resolved for the wrong nickname".into());
    }
    connection.close().await
}

/// delivers channel history in a batch
#[scenario(requires(caps = ["batch", "draft/chathistory"]))]
async fn batch(context: Scenario) -> ScenarioResult {
    history_page(context).await
}

/// rejects an invalid real name with a standard failure
#[scenario(requires(caps = ["standard-replies", "setname"]))]
async fn standard_replies(context: Scenario) -> ScenarioResult {
    if std::env::var("PAPO_TEST_SERVER").is_ok_and(|server| server != "ergo") {
        return Ok(libtest_mimic::Completion::ignored_with(
            "portable standard-replies failure is not set up",
        ));
    }
    let nick = context.unique();
    let mut connection = context.client(&nick).await?;
    connection
        .client
        .send(Message::new("SETNAME", [""]))
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut connection.events,
            |event| {
                matches!(event, Event::StandardReply(reply) if reply.kind == StandardReplyKind::Fail && reply.command == "SETNAME" && reply.code == "INVALID_REALNAME")
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    connection.close().await
}
