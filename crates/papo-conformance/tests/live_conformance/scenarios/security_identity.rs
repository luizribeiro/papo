use papo::Event;
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
    let alice_nick = alice.snapshot().nick().clone();
    let mallory_nick = mallory.snapshot().nick().clone();
    let channel = s.channel_name();
    bot.join(&channel).await?;
    alice.join(&channel).await?;
    mallory.join(&channel).await?;

    let bot_direct = bot.snapshot().nick().to_string();
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

    alice.quit().await?;
    bot.wait_for(|event| {
        event
            .as_quit()
            .filter(|quit| quit.actor.nick() == alice_nick.as_str())
    })
    .await?;
    mallory.set_nick(alice_nick.as_str()).await?;
    bot.wait_for(|event| {
        event.as_nick_changed().filter(|changed| {
            changed.actor.nick() == alice_nick.as_str()
                && changed.old == mallory_nick.as_str()
                && changed.new == alice_nick.as_str()
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

/// refuses an admin check and a guarded reply after an unseen private sender is replaced
#[scenario(covers = ["security.guard-proof.unseen-message-peer-handoff"])]
async fn unseen_private_sender_handoff(s: Scenario) -> papo_testing::Result<()> {
    match profile().as_str() {
        "soju-ergo" => {
            return s.skip("soju keeps the upstream user attached after downstream QUIT");
        }
        "ergo-default" | "ergo-persistent" => {
            return s.skip("Ergo reserves a logged-in user's nick for their account");
        }
        _ => {}
    }
    let mut bot = s.connect("bot").await?;
    let alice = s.connect_as_account("alice").await?;
    let mallory = s.connect("mallory").await?;
    let spare = s.connect("handoff").await?;
    let handoff = spare.snapshot().nick().clone().to_string();
    spare.quit().await?;
    alice.set_nick(&handoff).await?;
    let bot_nick = bot.snapshot().nick().clone();

    alice.privmsg(bot_nick.as_str(), "!ping").await?;
    let (admin, state) = bot
        .wait_for_with_state(|event| message_text(event, "!ping"))
        .await?;
    assert!(matches!(
        state.user(&admin.actor)?.map(papo::User::account),
        Some(papo::AccountStatus::LoggedIn(_))
    ));

    alice.quit().await?;
    mallory.set_nick(&handoff).await?;
    mallory.privmsg(bot_nick.as_str(), "!op mallory").await?;
    let (takeover, state) = bot
        .wait_for_with_state(|event| message_text(event, "!op mallory"))
        .await?;

    assert_eq!(
        state.user(&takeover.actor)?.map(papo::User::account),
        Some(&papo::AccountStatus::LoggedOut)
    );
    assert_ne!(takeover.actor, admin.actor);
    assert!(matches!(
        bot.reply(&admin, "op granted").await,
        Err(papo::Error::TargetGone)
    ));
    Ok(())
}

/// keeps a broadcast out of the replyable message event
#[scenario(covers = ["security.reply.broadcast-private"])]
async fn broadcast_is_a_server_notice(s: Scenario) -> papo_testing::Result<()> {
    let profile = profile();
    if !matches!(profile.as_str(), "solanum" | "unrealircd") {
        return s.skip("the selected profile does not preserve operator broadcast masks");
    }
    let oper = s.connect("oper").await?;
    let mut recipient = s.connect("recipient").await?;
    oper.oper(
        papo_test_server::OPER_NAME,
        papo::Secret::new(papo_test_server::OPER_PASSWORD),
    )
    .await?;

    let target = if profile == "solanum" {
        "$$*.papo.test"
    } else {
        "$*.papo.test"
    };
    oper.send_raw(papo::Line::privmsg(target, "deployment status requested"))
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
        papo::event::NoticeTarget::Broadcast(papo::event::Broadcast::Mask(_))
    ));
    assert_eq!(
        crate::source_nick(&broadcast.actor),
        Some(oper.snapshot().nick().as_str())
    );
    Ok(())
}

/// refuses a private reply when only a later JOIN reveals the sender was replaced
#[scenario(covers = ["security.recipient-1.live-join-handoff"])]
async fn private_reply_refuses_unseen_join_handoff(s: Scenario) -> papo_testing::Result<()> {
    let mut bot = s.connect("bot").await?;
    let bouncer = s.server.bouncer();
    let sender = |label| {
        if let Some(bouncer) = bouncer {
            let upstream = bouncer.upstream_addr();
            let endpoint = papo::Endpoint::plaintext(upstream.ip().to_string(), upstream.port())?;
            Ok(
                papo::Client::builder(endpoint, papo_test_server::unique(label))
                    .allow_insecure_auth(true),
            )
        } else {
            s.configure(label)
        }
    };
    let alice = s.connect_with(sender("alice")?.username("alice")).await?;
    let mut replacement = sender("mallory")?.username("mallory");
    if profile().starts_with("ergo-") || bouncer.is_some() {
        replacement = replacement.webirc(
            papo::WebIrc::new(
                "papo-gateway",
                papo::Secret::new(papo_test_server::WEBIRC_PASSWORD),
                "192.0.2.42".parse()?,
            )
            .hostname("handoff.papo.test"),
        );
    }
    let mut mallory = s.connect_with(replacement).await?;
    let channel = s.channel_name();
    bot.join(&channel).await?;
    let nick = alice.snapshot().nick().clone();
    alice
        .privmsg(bot.snapshot().nick().as_str(), "release status")
        .await?;
    let (request, before) = bot
        .wait_for_with_state(|event| message_text(event, "release status"))
        .await?;
    alice.quit().await?;
    mallory.set_nick(nick.as_str()).await?;
    assert_eq!(mallory.snapshot().nick(), nick.as_str());
    mallory.join(&channel).await?;
    let (newcomer, after) = bot
        .wait_for_with_state(|event| {
            event
                .as_joined()
                .filter(|joined| joined.actor.nick() == nick.as_str())
        })
        .await?;
    let original = before.user(&request.actor)?.unwrap();
    let replacement = after.user(&newcomer.actor)?.unwrap();
    assert_ne!(
        (original.username(), original.host()),
        (replacement.username(), replacement.host())
    );
    assert_ne!(request.actor, newcomer.actor);
    assert!(matches!(
        bot.reply(&request, "release approved").await,
        Err(papo::Error::TargetGone)
    ));
    assert!(matches!(
        bot.privmsg(&request.actor, "release approved").await,
        Err(papo::Error::TargetGone)
    ));
    mallory
        .expect_none(|event| message_text(event, "release approved"))
        .await?;
    Ok(())
}

/// keeps a private reply valid when either the sender or another user is watched
#[scenario(requires(isupport = ["MONITOR"]), covers = [
    "security.recipient-3.live-watch-keeps-reply"
])]
async fn private_reply_survives_watch(s: Scenario) -> papo_testing::Result<()> {
    let mut bot = s.connect("bot").await?;
    let mut alice = s.connect("alice").await?;
    let bob = s.connect("bob").await?;
    alice
        .privmsg(bot.snapshot().nick().as_str(), "release status")
        .await?;
    let request = bot
        .wait_for(|event| message_text(event, "release status"))
        .await?;
    let bob_nick = bob.snapshot().nick().to_string();
    let _bob_watch = bot.watch(&bob_nick);
    bot.wait_for(|event| {
        event
            .as_monitor_online()
            .filter(|online| online.subject.nick() == bob_nick.as_str())
    })
    .await?;
    bot.reply(&request, "release approved").await?;
    alice
        .wait_for(|event| message_text(event, "release approved"))
        .await?;

    let _alice_watch = request.actor.watch();
    let online = bot.wait_for(Event::as_monitor_online).await?;
    assert_eq!(online.subject, request.actor);
    bot.reply(&request, "release published").await?;
    alice
        .wait_for(|event| message_text(event, "release published"))
        .await?;
    Ok(())
}
