use papo::{CtcpReplies, Event};
use papo_conformance_macros::scenario;

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
    bob.wait_for(|event| event.as_ctcp().filter(|ctcp| ctcp.command == "VERSION"))
        .await?;
    alice
        .expect_none(|event| event.as_ctcp().filter(|ctcp| ctcp.reply))
        .await?;

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
                && raw.param(1) == Some("\u{1}VERSION\u{1}")
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
            event.as_ctcp().filter(|ctcp| {
                ctcp.reply
                    && ctcp.command == "VERSION"
                    && crate::source_nick(&ctcp.actor) == Some(bob_nick.as_ref())
            })
        })
        .await?;
    Ok(())
}
