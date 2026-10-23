use papo_bot::BotConfig;
use papo_conformance_macros::scenario;

use crate::Scenario;
use crate::scenarios::support::message_text;

fn profile() -> String {
    std::env::var("PAPO_TEST_PROFILE").unwrap_or_else(|_| "ergo-default".into())
}

/// refuses delayed private bot output after another user claims the sender's nick
#[scenario(
    requires(caps = ["account-tag"]),
    covers = ["security.bot.private-reply-after-nick-takeover"]
)]
async fn private_bot_reply_after_nick_takeover(s: Scenario) -> papo_testing::Result<()> {
    if matches!(
        profile().as_str(),
        "ergo-default" | "ergo-persistent" | "soju-ergo"
    ) {
        return s.skip("the selected profile reserves authenticated account nicknames");
    }
    let mut bot = s.connect("bot").await?;
    let alice = s.connect_as_account("alice").await?;
    let mut mallory = s.connect("mallory").await?;
    let alice_nick = alice.nick();
    let mallory_nick = mallory.nick();
    let channel = s.channel_name();
    bot.join(&channel).await?;
    alice.join(&channel).await?;
    mallory.join(&channel).await?;

    alice.privmsg(bot.nick(), "!lookup payroll").await?;
    let message = bot
        .wait_for(|event| message_text(event, "!lookup payroll"))
        .await?;
    let command = BotConfig::new("!")
        .command(&bot, &message)
        .expect("the private command is parsed");
    assert!(command.account().is_some());

    alice.quit("handoff").await?;
    bot.wait_for(|event| event.as_quit().filter(|quit| quit.nick == alice_nick))
        .await?;
    mallory.set_nick(&alice_nick).await?;
    bot.wait_for(|event| {
        event
            .as_nick_changed()
            .filter(|changed| changed.old == mallory_nick && changed.new == alice_nick)
    })
    .await?;

    assert!(matches!(
        command.reply("SECRET payroll").await,
        Err(papo::Error::TargetGone)
    ));
    assert!(matches!(
        command.private_notice("SECRET audit").await,
        Err(papo::Error::TargetGone)
    ));
    mallory
        .expect_none(|event| {
            event.as_message().filter(|message| {
                matches!(message.text.as_str(), "SECRET payroll" | "SECRET audit")
            })
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
        papo::event::ServerNoticeTarget::Mask(_)
    ));
    assert_eq!(
        crate::source_nick(&broadcast.actor),
        Some(oper.nick().as_ref())
    );
    Ok(())
}
