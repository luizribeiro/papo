use papo::{CtcpReplies, Event};
use papo_conformance_macros::scenario;

use super::support::wait_channel_mode;
use crate::Scenario;

fn profile() -> String {
    std::env::var("PAPO_TEST_PROFILE").unwrap_or_else(|_| "ergo-default".into())
}

/// rejoins a channel under its renamed address after reconnecting
#[scenario(
    requires(caps = ["draft/channel-rename"]),
    covers = ["security.rejoin.channel-rename"]
)]
async fn reconnect_follows_channel_rename(s: Scenario) -> papo_testing::Result<()> {
    let old = s.channel_name();
    let renamed = format!("{old}-archive");
    let mut alice = s
        .connect_with(
            s.configure("alice")?
                .request_caps(["draft/channel-rename"])
                .autojoin([&old]),
        )
        .await?;
    alice
        .wait_for(|event| {
            event.as_joined().filter(|joined| {
                joined.channel.as_ref() == old && joined.authorship == papo::Authorship::Own
            })
        })
        .await?;

    alice
        .rename_channel(&old, &renamed, "archive the project")
        .await?;
    alice.wait_for(Event::as_channel_renamed).await?;
    alice.reconnect().await?;
    alice.wait_for(Event::as_disconnected).await?;
    alice.wait_for(Event::as_connected).await?;
    let joined = alice
        .wait_for(|event| {
            event.as_joined().filter(|joined| {
                joined.authorship == papo::Authorship::Own && joined.channel.as_ref() == renamed
            })
        })
        .await?;
    assert_eq!(joined.channel.as_ref(), renamed);
    assert!(alice.state().channel(&old).is_none());
    assert!(alice.state().channel(&renamed).is_some());
    Ok(())
}

/// completes a labelled re-JOIN acknowledged without an echoed JOIN
#[scenario(
    requires(caps = ["labeled-response"]),
    covers = ["security.r1.labelled-rejoin-routing"]
)]
async fn labelled_rejoin_ack_completes(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let channel = s.channel_name();
    alice.join(&channel).await?;
    alice.join(&channel).await?;
    assert!(alice.state().channel(&channel).is_some());
    Ok(())
}

/// does not restore a channel parted after a labelled re-JOIN
#[scenario(
    requires(caps = ["labeled-response"]),
    covers = ["security.r2.no-stale-rejoin-after-part"]
)]
async fn parted_rejoin_is_not_restored(s: Scenario) -> papo_testing::Result<()> {
    let channel = s.channel_name();
    let mut alice = s.connect("alice").await?;
    alice.join(&channel).await?;
    alice.join(&channel).await?;
    alice.part(&channel, "leaving the private room").await?;
    alice
        .wait_for(|event| {
            event.as_parted().filter(|parted| {
                parted.authorship == papo::Authorship::Own && parted.channel.as_ref() == channel
            })
        })
        .await?;

    alice.reconnect().await?;
    alice.wait_for(Event::as_disconnected).await?;
    alice.wait_for(Event::as_connected).await?;
    assert!(alice.state().channel(&channel).is_none());
    alice
        .expect_none(|event| {
            event.as_joined().filter(|joined| {
                joined.authorship == papo::Authorship::Own && joined.channel.as_ref() == channel
            })
        })
        .await?;
    Ok(())
}

/// keeps late members of a refused batch from causing live side effects
#[scenario(covers = ["security.h1.abandoned-batch-members-not-live"])]
async fn abandoned_batch_members_stay_replay_only(_s: Scenario) -> papo_testing::Result<()> {
    let (client, _events, mut wire) = papo_testing::wire(
        papo::Client::builder("irc://offline.test")
            .nick("papo-wire")
            .rate_limit(papo::RateLimit::unlimited()),
    );
    wire.register(&["batch"]).await;
    wire.server(":papo-wire!~papo@localhost JOIN #private")
        .await;
    for index in 0..64 {
        wire.server(format!("BATCH +held{index} vendor/sync")).await;
    }
    wire.server("BATCH +refused draft/event-playback #private")
        .await;
    wire.server("@batch=refused :alice!~alice@staff.example PRIVMSG papo-wire :\u{1}VERSION\u{1}")
        .await;
    wire.server("@batch=refused :papo-wire!~papo@localhost PART #private :old playback")
        .await;

    let ping_client = client.clone();
    let ping = tokio::spawn(async move { ping_client.ping().await });
    let sent = wire.next_sent().await;
    let token = sent
        .strip_prefix("PING ")
        .unwrap_or_else(|| panic!("late replay traffic caused an outgoing line: {sent}"));
    wire.server(format!(":offline.test PONG offline.test :{token}"))
        .await;
    ping.await.unwrap()?;
    assert!(client.state().channel("#private").is_some());
    Ok(())
}

/// keeps a message refusal separate from a concurrent unlabelled PART
#[scenario(covers = ["security.r4.unlabelled-command-numerics"])]
async fn message_refusal_does_not_fail_part(s: Scenario) -> papo_testing::Result<()> {
    if profile() != "solanum" {
        return s.skip("the selected profile does not exercise Solanum's unlabelled 404 ordering");
    }
    let alice = s.connect("alice").await?;
    let mut bob = s
        .connect_with(
            s.configure("bob")?
                .without_caps([papo::cap::LABELED_RESPONSE, papo::cap::ECHO_MESSAGE]),
        )
        .await?;
    let channel = s.channel_name();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    alice.mode(&channel).add('m').await?;
    wait_channel_mode(&mut bob, &channel, 'm').await?;

    let (message, part) = tokio::join!(
        bob.privmsg(&channel, "blocked while unvoiced"),
        bob.part(&channel, "leaving after the rejected message"),
    );
    message?;
    part?;
    let refusal = bob
        .wait_for(|event| event.as_other().filter(|line| line.numeric() == Some(404)))
        .await?;
    assert_eq!(refusal.as_ref().param(1), Some(channel.as_str()));
    assert!(bob.state().channel(&channel).is_none());
    Ok(())
}

/// does not answer a CTCP query replayed from bouncer history
#[scenario(
    requires(caps = ["batch"]),
    covers = ["security.ctcp.no-reply-to-playback"]
)]
async fn ctcp_playback_has_no_automatic_reply(s: Scenario) -> papo_testing::Result<()> {
    if profile() != "soju-ergo" {
        return s.skip("the selected profile does not expose bouncer playback");
    }
    let mut alice = s.connect("alice").await?;
    let mut bob = s
        .connect_with(s.configure("bob")?.ctcp_replies(CtcpReplies::off()))
        .await?;
    let bob_nick = bob.nick();

    alice
        .send_raw(papo::Line::privmsg(bob_nick.as_ref(), "\u{1}VERSION\u{1}"))
        .await?;
    alice.ping().await?;
    bob.wait_for(|event| {
        event
            .as_ctcp_query()
            .filter(|ctcp| ctcp.command == "VERSION")
    })
    .await?;
    alice.expect_none(Event::as_ctcp_reply).await?;

    let phone = s
        .connect_with(
            s.configure_account_session("bob-phone", bob_nick.to_string())?
                .ctcp_replies(CtcpReplies::default().version(Some("papo security test"))),
        )
        .await?;
    let page = phone
        .history(alice.nick().as_ref())
        .latest()
        .limit(10)
        .await?;
    assert!(page.items.iter().any(|item| {
        matches!(item, papo::Event::Other(raw) if {
            raw.command() == "PRIVMSG"
                && raw.get(1) == Some("\u{1}VERSION\u{1}")
                && raw.source().is_some_and(|source| {
                    matches!(
                        source,
                        papo_proto::Source::User { nick, .. }
                            if nick == alice.nick().as_ref()
                    )
                })
        })
    }));
    alice
        .expect_none(|event| {
            event.as_ctcp_reply().filter(|ctcp| {
                ctcp.command == "VERSION"
                    && crate::source_nick(&ctcp.actor) == Some(bob_nick.as_ref())
            })
        })
        .await?;
    Ok(())
}
