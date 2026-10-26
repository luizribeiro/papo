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
        .connect_with(s.configure("alice")?.autojoin([&old]))
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
    alice
        .send_raw(
            papo::Line::new("PART").with_params([channel.as_str(), "leaving the private room"]),
        )
        .await?;
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
        papo::Client::builder("irc://offline.test", "papo-wire")
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
                .disable(papo::Feature::LabeledResponse)
                .disable(papo::Feature::EchoMessages),
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

/// keeps wildcard WHO rows separate from a concurrent channel WHO
#[scenario(covers = ["security.r5.wildcard-who-routing"])]
async fn wildcard_who_stays_separate_from_channel_who(s: Scenario) -> papo_testing::Result<()> {
    if profile() != "solanum" {
        return s.skip("the selected profile does not exercise Solanum's unlabelled WHO replies");
    }
    let alice = s.connect("alice").await?;
    let bob = s
        .connect_with(s.configure("bob")?.disable(papo::Feature::LabeledResponse))
        .await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    bob.ping().await?;

    let (wildcard, channel_rows) = tokio::join!(bob.who("*"), bob.who(&channel));
    let wildcard = wildcard?;
    let channel_rows = channel_rows?;
    assert!(wildcard.iter().any(|entry| entry.nick == alice_nick));
    assert!(
        channel_rows.iter().any(|entry| entry.nick == alice_nick),
        "wildcard WHO rows: {wildcard:?}; channel WHO rows: {channel_rows:?}"
    );
    Ok(())
}

/// keeps early account creation separate from a later request
#[scenario(covers = ["security.r6.early-account-request-id"])]
async fn early_account_creation_reserves_its_request_id(_s: Scenario) -> papo_testing::Result<()> {
    let (client, _events, mut wire) = papo_testing::wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .allow_insecure_auth(true)
            .rate_limit(papo::RateLimit::unlimited()),
    );
    assert_eq!(wire.next_sent().await, "CAP LS 302");
    assert_eq!(wire.next_sent().await, "NICK papo-wire");
    assert!(wire.next_sent().await.starts_with("USER "));
    wire.server(
        ":offline.test CAP * LS :draft/account-registration=before-connect labeled-response",
    )
    .await;
    assert_eq!(
        wire.next_sent().await,
        "CAP REQ :labeled-response draft/account-registration"
    );

    let mut registration =
        Box::pin(client.create_account(papo::NewAccount::new("unique-registration-password")));
    tokio::select! {
        biased;
        result = &mut registration => panic!("early registration finished before CAP ACK: {result:?}"),
        () = std::future::ready(()) => {}
    }
    let mut whois = Box::pin(client.whois("alice"));
    tokio::select! {
        biased;
        result = &mut whois => panic!("WHOIS finished before a server reply: {result:?}"),
        () = std::future::ready(()) => {}
    }
    assert_eq!(wire.next_sent().await, "WHOIS alice");

    wire.server(":offline.test CAP * ACK :labeled-response draft/account-registration")
        .await;
    assert_eq!(
        wire.next_sent().await,
        "@label=0 REGISTER * * unique-registration-password"
    );
    wire.server("@label=0 :offline.test REGISTER SUCCESS alice :Account created")
        .await;
    assert_eq!(wire.next_sent().await, "CAP END");
    wire.server(":offline.test 311 papo-wire alice ~alice staff.example * :Alice Example")
        .await;
    wire.server(":offline.test 318 papo-wire alice :End of /WHOIS list")
        .await;

    assert!(matches!(
        registration.await?,
        papo::AccountCreation::Created { .. }
    ));
    assert_eq!(whois.await?.nick.as_ref(), "alice");
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
                .ctcp_replies(CtcpReplies::default().version("papo security test")),
        )
        .await?;
    let page = phone
        .history(alice.nick().as_ref())
        .latest()
        .limit(10)
        .await?;
    assert!(page.items.iter().any(|item| {
        matches!(item, papo::Event::CtcpQuery(query)
            if query.command == "VERSION"
                && query.actor.nick().as_ref() == alice.nick().as_ref())
    }));
    alice
        .expect_none(|event| {
            event
                .as_ctcp_reply()
                .filter(|ctcp| ctcp.command == "VERSION" && ctcp.actor.nick() == &bob_nick)
        })
        .await?;
    Ok(())
}
