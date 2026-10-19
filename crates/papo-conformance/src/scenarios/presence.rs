use papo::{
    Account, AccountName, AccountRegistration, Away, Error, Event, NewAccount, Origin, Refusal,
    Source,
};
use papo_conformance_macros::scenario;

use crate::Scenario;

/// delivers an authenticated sender account
#[scenario(
    requires(caps = ["account-tag", "account-notify"]),
    covers = ["ircv3.account-tag"]
)]
async fn account_tag(context: Scenario) -> papo_testing::Result<()> {
    let alice = context.connect_as_account("alice").await?;
    let mut bob = context.connect("bob").await?;
    let account = alice.state().account().unwrap().clone();
    let alice_nick = alice.nick();
    let channel = context.channel_name();
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    let text = "authenticated hello from alice";
    let sent = alice.privmsg(&channel, text).await?;
    let message = bob
        .wait_for(|event| event.as_message().filter(|message| message.text() == text))
        .await?;

    assert_eq!(message.text(), text);
    assert_eq!(
        message.sender.as_ref().and_then(Source::nick),
        Some(alice_nick.as_ref())
    );
    assert_eq!(message.target(), channel);
    assert_eq!(message.id(), sent.id.as_ref());
    assert_eq!(message.server_time, sent.time);
    assert_eq!(message.account(), Some(account.as_ref()));
    assert_eq!(message.origin(), Origin::Live);
    assert!(sent.fallbacks.is_empty());
    assert!(matches!(
        bob.state().user(&alice_nick).unwrap().account(),
        Account::LoggedIn(observed) if observed == &account
    ));
    bob.expect_none(|event| event.as_message().filter(|message| message.text() == text))
        .await?;
    Ok(())
}

/// notifies a shared-channel peer when a user authenticates an account
#[scenario(
    requires(caps = ["account-notify", "draft/account-registration"]),
    covers = ["ircv3.account-notify"]
)]
async fn account_notify(context: Scenario) -> papo_testing::Result<()> {
    let mut alice = context
        .connect_with(
            context
                .configure("alice")?
                .request_caps(["draft/account-registration"]),
        )
        .await?;
    let mut bob = context.connect("bob").await?;
    let alice_nick = alice.nick();
    let channel = context.channel_name();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    let account = AccountName::from(alice_nick.as_ref());

    let outcome = alice
        .register_account(NewAccount::new("q7Hx-2mZp_9vLcT4wRfB").account(account.clone()))
        .await?;
    assert!(matches!(
        outcome,
        AccountRegistration::Registered { account: ref registered } if registered == &account
    ));
    let changed = bob.wait_for(Event::as_account_changed).await?;

    assert_eq!(changed.nick, alice_nick);
    assert_eq!(changed.account.as_ref(), Some(&account));
    assert_eq!(alice.state().account(), Some(&account));
    assert!(matches!(
        bob.state().user(&alice_nick).unwrap().account(),
        Account::LoggedIn(observed) if observed == &account
    ));
    alice.wait_for(Event::as_account_registration).await?;
    bob.expect_none(Event::as_account_changed).await?;
    Ok(())
}

/// confirms away state locally and notifies channel peers of both transitions
#[scenario(
    requires(caps = ["away-notify"]),
    covers = ["base.away.self-ack", "ircv3.away-notify"]
)]
async fn away_notify(context: Scenario) -> papo_testing::Result<()> {
    let mut bob = context.connect("bob").await?;
    let alice = context.connect("alice").await?;
    let channel = context.channel_name();
    let alice_nick = alice.nick();
    bob.join(&channel).await?;
    alice.join(&channel).await?;
    let away = "reviewing the IRCv3 specification";
    alice.set_away(away).await?;
    let changed = bob.wait_for(Event::as_away_changed).await?;

    assert_eq!(changed.nick, alice_nick);
    assert_eq!(changed.message.as_deref(), Some(away));
    assert!(alice.state().away());
    assert_eq!(alice.state().away_message(), Some(away));
    assert_eq!(
        bob.state().user(&alice_nick).unwrap().away(),
        &Away::Away {
            message: Some(away.to_owned())
        }
    );

    alice.clear_away().await?;
    let returned = bob.wait_for(Event::as_away_changed).await?;

    assert_eq!(returned.nick, alice_nick);
    assert_eq!(returned.message, None);
    assert!(!alice.state().away());
    assert_eq!(alice.state().away_message(), None);
    assert_eq!(
        bob.state().user(&alice_nick).unwrap().away(),
        &Away::Present
    );
    bob.expect_none(Event::as_away_changed).await?;
    Ok(())
}

/// delivers a private message to an away user without changing away state
#[scenario(covers = ["base.away.privmsg-to-away-user"])]
async fn away_private_message(context: Scenario) -> papo_testing::Result<()> {
    let mut alice = context.connect("alice").await?;
    let bob = context.connect("bob").await?;
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    let away = "reviewing private-message delivery";
    alice.set_away(away).await?;

    let text = "the review can wait until you return";
    let sent = bob.privmsg(alice_nick.clone(), text).await?;
    let message = alice
        .wait_for(|event| event.as_message().filter(|message| message.text() == text))
        .await?;

    assert_eq!(
        message.sender.as_ref().and_then(Source::nick),
        Some(bob_nick.as_ref())
    );
    assert_eq!(message.target(), alice_nick.as_ref());
    assert_eq!(message.text(), text);
    assert_eq!(message.id(), sent.id.as_ref());
    assert_eq!(message.server_time, sent.time);
    assert_eq!(message.origin(), Origin::Live);
    assert!(alice.state().away());
    assert_eq!(alice.state().away_message(), Some(away));
    assert!(sent.fallbacks.is_empty());
    alice
        .expect_none(|event| event.as_message().filter(|message| message.text() == text))
        .await?;
    Ok(())
}

/// updates real-name state for the sender and every shared-channel peer
#[scenario(requires(caps = ["setname"]), covers = ["ircv3.setname"])]
async fn setname(context: Scenario) -> papo_testing::Result<()> {
    let mut alice = context.connect("alice").await?;
    let mut bob = context.connect("bob").await?;
    let channel = context.channel_name();
    let alice_nick = alice.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    let realname = "Papo Conformance Client";
    alice.set_realname(realname).await?;
    let own = alice.wait_for(Event::as_realname_changed).await?;
    let observed = bob.wait_for(Event::as_realname_changed).await?;

    assert_eq!(own.nick, alice_nick);
    assert_eq!(own.realname, realname);
    assert_eq!(observed.nick, alice_nick);
    assert_eq!(observed.realname, realname);
    assert_eq!(
        alice.state().user(&alice_nick).unwrap().realname(),
        Some(realname)
    );
    assert_eq!(
        bob.state().user(&alice_nick).unwrap().realname(),
        Some(realname)
    );
    alice.expect_none(Event::as_realname_changed).await?;
    bob.expect_none(Event::as_realname_changed).await?;
    Ok(())
}

/// updates username and host state for the target and a shared-channel peer
#[scenario(
    requires(caps = ["chghost"]),
    covers = ["base.oper.success", "ircv3.chghost"]
)]
async fn chghost(context: Scenario) -> papo_testing::Result<()> {
    let Ok(oper) = std::env::var("PAPO_TEST_OPER") else {
        return context.skip("PAPO_TEST_OPER is not set up");
    };
    let Some((oper_name, password)) = oper.split_once(':') else {
        return Err("PAPO_TEST_OPER must contain name:password".into());
    };
    let mut alice = context.connect("alice").await?;
    let mut bob = context.connect("bob").await?;
    let channel = context.channel_name();
    let bob_nick = bob.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    let username = bob
        .state()
        .user(&bob_nick)
        .unwrap()
        .username()
        .unwrap()
        .to_owned();

    alice.oper(oper_name, password).await?;
    alice
        .send_command("CHGHOST", [bob_nick.as_ref(), "conformance.example"])
        .await?;
    let own = bob.wait_for(Event::as_host_changed).await?;
    let observed = alice.wait_for(Event::as_host_changed).await?;

    for changed in [&own, &observed] {
        assert_eq!(changed.nick, bob_nick);
        assert_eq!(changed.user, username);
        assert_eq!(changed.host, "conformance.example");
    }
    for peer in [&alice, &bob] {
        let state = peer.state();
        let user = state.user(&bob_nick).unwrap();
        assert_eq!(user.username(), Some(username.as_str()));
        assert_eq!(user.host(), Some("conformance.example"));
    }
    alice.expect_none(Event::as_host_changed).await?;
    bob.expect_none(Event::as_host_changed).await?;
    Ok(())
}

/// tracks both online and offline transitions with extended identity fields
#[scenario(requires(
    caps = ["extended-monitor", "away-notify"],
    isupport = ["MONITOR"]
), covers = [
    "ircv3.extended-monitor",
    "ircv3.monitor.online",
    "ircv3.monitor.offline"
])]
async fn monitor(context: Scenario) -> papo_testing::Result<()> {
    let mut alice = context.connect("alice").await?;
    let bob = context.connect("bob").await?;
    let bob_nick = bob.nick();
    alice.monitor().add([bob_nick.clone()]).await?;
    let online = alice.wait_for(Event::as_monitor_online).await?;

    assert_eq!(online.nick, bob_nick);
    let state = alice.state();
    let monitored = state.monitor_online().collect::<Vec<_>>();
    assert_eq!(monitored.len(), 1);
    assert_eq!(monitored[0].nick(), &bob_nick);
    assert_eq!(monitored[0].username(), online.user.as_deref());
    assert_eq!(monitored[0].host(), online.host.as_deref());

    let away = "reading the monitor specification";
    bob.set_away(away).await?;
    let changed = alice.wait_for(Event::as_away_changed).await?;

    assert_eq!(changed.nick, bob_nick);
    assert_eq!(changed.message.as_deref(), Some(away));
    assert_eq!(
        alice.state().user(&bob_nick).unwrap().away(),
        &Away::Away {
            message: Some(away.to_owned())
        }
    );

    let new_nick = format!("r{bob_nick}");
    bob.set_nick(new_nick.clone()).await?;
    let offline = alice.wait_for(Event::as_monitor_offline).await?;

    assert_eq!(offline.nick, bob_nick);
    assert_eq!(bob.state().nick(), new_nick.as_str());
    assert!(
        alice
            .state()
            .monitor_online()
            .all(|user| user.nick() != &bob_nick)
    );
    alice.expect_none(Event::as_monitor_offline).await?;
    Ok(())
}

/// exposes configured bot identity through mode, WHOIS, state, and message tags
#[scenario(
    requires(caps = ["message-tags"], isupport = ["BOT"]),
    covers = [
        "base.mode.user.bot",
        "base.whois.bot-flag",
        "base.whois.user",
        "ircv3.bot-mode",
        "tag.bot"
    ]
)]
async fn bot_mode(context: Scenario) -> papo_testing::Result<()> {
    let mut alice = context
        .connect_with(context.configure("alice")?.bot())
        .await?;
    let mut bob = context.connect("bob").await?;
    let alice_nick = alice.nick();
    let channel = context.channel_name();
    let changed = alice
        .wait_for(|event| {
            event.as_mode_changed().filter(|changed| {
                changed.target.as_ref() == alice_nick.as_ref()
                    && changed.changes.iter().any(|change| change.mode == 'B')
            })
        })
        .await?;

    assert_eq!(changed.target.as_ref(), alice_nick.as_ref());
    let bot_change = changed
        .changes
        .iter()
        .find(|change| change.mode == 'B')
        .unwrap();
    assert!(bot_change.set);
    assert!(alice.state().user_modes().any(|mode_| mode_ == 'B'));

    alice.join(&channel).await?;
    bob.join(&channel).await?;
    let whois = bob.whois(alice_nick.clone()).await?;

    assert_eq!(whois.nick, alice_nick);
    assert!(whois.user.as_ref().is_some_and(|user| !user.is_empty()));
    assert!(whois.host.as_ref().is_some_and(|host| !host.is_empty()));
    assert!(whois.realname.as_ref().is_some_and(|name| !name.is_empty()));
    assert!(whois.is_bot);

    let text = "automated conformance report";
    alice.privmsg(&channel, text).await?;
    let message = bob
        .wait_for(|event| event.as_message().filter(|message| message.text() == text))
        .await?;

    assert_eq!(
        message.sender.as_ref().and_then(Source::nick),
        Some(alice_nick.as_ref())
    );
    assert_eq!(message.target(), channel);
    assert_eq!(message.text(), text);
    assert!(message.is_bot());
    bob.expect_none(|event| event.as_message().filter(|message| message.text() == text))
        .await?;
    Ok(())
}

/// reports channel identity and away state through WHO and WHOIS
#[scenario(covers = [
    "base.away.reflected-in-whois",
    "base.who.plain-reply",
    "base.who.channel-scoped",
    "base.who.away-flag",
    "base.whois.user",
    "base.whois.idle",
    "base.whois.channels",
    "base.whois.err-no-such-nick"
])]
async fn who_and_whois(context: Scenario) -> papo_testing::Result<()> {
    let alice = context.connect("alice").await?;
    let bob = context.connect("bob").await?;
    let channel = context.channel_name();
    let alice_nick = alice.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    let away = "reading WHO reply fields";
    alice.set_away(away).await?;

    let who = bob.who(channel.clone()).await?;
    let alice_who = who.iter().find(|entry| entry.nick == alice_nick).unwrap();

    assert_eq!(
        alice_who.channel.as_ref().map(AsRef::as_ref),
        Some(channel.as_str())
    );
    assert!(!alice_who.user.is_empty());
    assert!(!alice_who.host.is_empty());
    assert_eq!(alice_who.realname, "papo test peer");
    assert!(alice_who.away);
    assert!(!alice_who.is_oper);
    assert!(!alice_who.is_bot);
    assert_eq!(alice_who.prefixes, ['@']);

    // Plain WHO's numeric fields are the behavior under test, without WHOX rewriting them.
    let plain = bob.request_command("WHO", [&channel]).await?;
    let row = plain
        .iter()
        .find(|line| line.numeric() == Some(352) && line.param(5) == Some(alice_nick.as_ref()))
        .unwrap();
    assert_eq!(row.param(1), Some(channel.as_str()));
    assert!(row.param(2).is_some_and(|user| !user.is_empty()));
    assert!(row.param(3).is_some_and(|host| !host.is_empty()));
    assert!(row.param(4).is_some_and(|server| !server.is_empty()));
    assert!(row.param(6).is_some_and(|flags| flags.contains('G')));
    assert!(row.param(7).is_some_and(|hop_realname| {
        hop_realname
            .split_once(' ')
            .is_some_and(|(_, realname)| realname == "papo test peer")
    }));
    assert!(
        plain
            .iter()
            .any(|line| { line.numeric() == Some(315) && line.param(1) == Some(channel.as_str()) })
    );

    let whois = bob.whois(alice_nick.clone()).await?;

    assert_eq!(whois.nick, alice_nick);
    assert!(whois.user.as_ref().is_some_and(|user| !user.is_empty()));
    assert!(whois.host.as_ref().is_some_and(|host| !host.is_empty()));
    assert_eq!(whois.realname.as_deref(), Some("papo test peer"));
    assert!(whois.idle.is_some());
    assert_eq!(whois.away.as_deref(), Some(away));
    assert!(
        whois
            .channels
            .iter()
            .any(|joined| joined.name.as_ref() == channel)
    );

    let Error::Server(refusal) = bob.whois("missing-presence-target").await.unwrap_err() else {
        panic!("unknown WHOIS target did not return a server refusal");
    };
    assert_eq!(refusal.kind(), Refusal::NoSuchNick);
    assert_eq!(refusal.code(), "401");
    Ok(())
}

/// reports the target's server through WHOIS
#[scenario(covers = ["base.whois.server"])]
async fn whois_server(context: Scenario) -> papo_testing::Result<()> {
    let alice = context.connect("alice").await?;
    let bob = context.connect("bob").await?;
    let alice_nick = alice.nick();

    let whois = bob.whois(alice_nick.clone()).await?;

    assert_eq!(whois.nick, alice_nick);
    assert!(
        whois
            .server
            .as_ref()
            .is_some_and(|server| !server.is_empty())
    );
    Ok(())
}

/// reports a client's visible host or certificate fingerprint through WHOIS
#[scenario(covers = ["base.whois.host-cert"])]
async fn whois_host_certificate(context: Scenario) -> papo_testing::Result<()> {
    let alice = context.connect("alice").await?;
    let alice_nick = alice.nick();

    // The host and certificate numerics have no richer public result type.
    let lines = alice
        .request_command("WHOIS", [alice_nick.as_ref()])
        .await?;
    let detail = lines
        .iter()
        .find(|line| matches!(line.numeric(), Some(276 | 378)))
        .unwrap();

    assert_eq!(detail.param(1), Some(alice_nick.as_ref()));
    assert!(detail.param(2).is_some_and(|value| !value.is_empty()));
    assert!(
        lines.iter().any(|line| {
            line.numeric() == Some(318) && line.param(1) == Some(alice_nick.as_ref())
        })
    );
    Ok(())
}
