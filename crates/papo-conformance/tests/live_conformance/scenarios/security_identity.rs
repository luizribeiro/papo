use papo_conformance_macros::scenario;

use crate::Scenario;
use crate::scenarios::support::message_text;

fn profile() -> String {
    std::env::var("PAPO_TEST_PROFILE").unwrap_or_else(|_| "ergo-default".into())
}

/// refuses a delayed private reply after another user claims the sender's nick
#[scenario(covers = ["security.bot.private-reply-after-nick-takeover"])]
async fn private_reply_after_nick_takeover(s: Scenario) -> papo_testing::Result<()> {
    if profile() == "soju-ergo" {
        return s.skip("soju keeps the upstream user attached after downstream QUIT");
    }
    let mut bot = s.connect("bot").await?;
    let mut alice = s.connect("alice").await?;
    let mut mallory = s.connect("mallory").await?;
    let alice_nick = alice.nick();
    let mallory_nick = mallory.nick();
    let channel = s.channel_name();
    bot.join(&channel).await?;
    alice.join(&channel).await?;
    mallory.join(&channel).await?;

    let bot_direct = alice.direct(bot.nick().as_ref()).await?;
    alice.privmsg(&bot_direct, "control request").await?;
    let control = bot
        .wait_for(|event| message_text(event, "control request"))
        .await?;
    bot.reply(&control, "control reply").await?;
    alice
        .wait_for(|event| message_text(event, "control reply"))
        .await?;

    alice.privmsg(&bot_direct, "release credentials").await?;
    let delayed = bot
        .wait_for(|event| message_text(event, "release credentials"))
        .await?;

    alice.quit("handoff").await?;
    bot.wait_for(|event| {
        event
            .as_quit()
            .filter(|quit| quit.actor.nick() == alice_nick.as_ref())
    })
    .await?;
    mallory.set_nick(alice_nick.as_ref()).await?;
    bot.wait_for(|event| {
        event.as_nick_changed().filter(|changed| {
            changed.actor.nick() == mallory_nick.as_ref() && changed.new == alice_nick.as_ref()
        })
    })
    .await?;

    assert!(matches!(
        bot.reply(&delayed, "SECRET payroll").await,
        Err(papo::Error::TargetGone)
    ));
    mallory
        .expect_none(|event| {
            event
                .as_message()
                .filter(|message| message.text == "SECRET payroll")
        })
        .await?;
    Ok(())
}

/// keeps a broadcast out of the replyable message event
#[scenario(covers = ["security.reply.broadcast-private"])]
async fn broadcast_is_a_server_notice(s: Scenario) -> papo_testing::Result<()> {
    if profile() != "solanum" {
        return s.skip("the selected profile does not preserve operator broadcast masks");
    }
    let oper = s.connect("oper").await?;
    let mut recipient = s.connect("recipient").await?;
    oper.oper(papo_test_server::OPER_NAME, papo_test_server::OPER_PASSWORD)
        .await?;

    oper.send_raw(papo::Line::privmsg(
        "$$*.papo.test",
        "deployment status requested",
    ))
    .await?;
    let broadcast = recipient
        .wait_for(|event| {
            event
                .as_server_notice()
                .filter(|notice| notice.text == "deployment status requested")
        })
        .await?;
    assert!(matches!(
        broadcast.to,
        papo::event::Audience::Broadcast(papo::event::Broadcast::Mask(_))
    ));
    assert_eq!(
        crate::source_nick(&broadcast.actor),
        Some(oper.nick().as_ref())
    );
    Ok(())
}
