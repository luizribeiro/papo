use papo::{Account, AccountCreation, Away, Error, Event, NewAccount, Origin, Refusal};
use papo_conformance_macros::scenario;

use super::support::{assert_refusal, subject_name};
use crate::Scenario;

/// delivers an authenticated sender account
#[scenario(
    requires(caps = ["account-tag", "account-notify"]),
    covers = ["ircv3.account-tag"]
)]
async fn account_tag(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect_as_account("alice").await?;
    let mut bob = s.connect("bob").await?;
    let account = alice.state().account().unwrap().clone();
    let alice_nick = alice.nick();
    let channel = s.channel_name();
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    let text = "authenticated hello from alice";
    let sent = alice.privmsg(&channel, text).await?;
    let message = bob
        .wait_for(|event| event.as_message().filter(|message| message.text() == text))
        .await?;

    assert_eq!(message.text(), text);
    assert_eq!(
        crate::source_nick(&message.actor),
        Some(alice_nick.as_ref())
    );
    assert_eq!(
        crate::scenarios::support::conversation_name(&message.conversation),
        Some(channel.as_str())
    );
    assert_eq!(message.id(), sent.id.as_ref());
    assert_eq!(Some(message.time), sent.time);
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
async fn account_notify(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s
        .connect_with(
            s.configure("alice")?
                .request_caps(["draft/account-registration"]),
        )
        .await?;
    let mut bob = s.connect("bob").await?;
    let alice_nick = alice.nick();
    let channel = s.channel_name();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    let account = alice.state().casemapping().account(alice_nick.as_ref());

    let outcome = alice
        .create_account(NewAccount::new("q7Hx-2mZp_9vLcT4wRfB").account(account.clone()))
        .await?;
    assert!(matches!(
        outcome,
        AccountCreation::Created { account: ref registered } if registered == &account
    ));
    let changed = bob.wait_for(Event::as_account_changed).await?;

    assert_eq!(changed.nick, alice_nick);
    assert_eq!(changed.account.as_ref(), Some(&account));
    assert_eq!(alice.state().account(), Some(&account));
    assert!(matches!(
        bob.state().user(&alice_nick).unwrap().account(),
        Account::LoggedIn(observed) if observed == &account
    ));
    alice.wait_for(Event::as_account_creation).await?;
    bob.expect_none(Event::as_account_changed).await?;
    Ok(())
}

/// confirms away state locally and notifies channel peers of both transitions
#[scenario(
    requires(caps = ["away-notify"]),
    covers = ["base.away.self-ack", "ircv3.away-notify"]
)]
async fn away_notify(s: Scenario) -> papo_testing::Result<()> {
    let mut bob = s.connect("bob").await?;
    let alice = s.connect("alice").await?;
    let channel = s.channel_name();
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
async fn away_private_message(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let bob = s.connect("bob").await?;
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    let away = "reviewing private-message delivery";
    alice.set_away(away).await?;

    let text = "the review can wait until you return";
    let sent = bob.privmsg(alice_nick.clone(), text).await?;
    let message = alice
        .wait_for(|event| event.as_message().filter(|message| message.text() == text))
        .await?;

    assert_eq!(crate::source_nick(&message.actor), Some(bob_nick.as_ref()));
    assert_eq!(
        crate::scenarios::support::conversation_name(&message.conversation),
        Some(bob_nick.as_ref())
    );
    assert_eq!(message.text(), text);
    assert_eq!(message.id(), sent.id.as_ref());
    assert_eq!(Some(message.time), sent.time);
    assert_eq!(message.origin(), Origin::Live);
    assert!(alice.state().away());
    assert_eq!(alice.state().away_message(), Some(away));
    assert!(sent.fallbacks.is_empty());
    alice
        .expect_none(|event| event.as_message().filter(|message| message.text() == text))
        .await?;
    Ok(())
}

/// announces away state during registration when pre-away is negotiated
#[scenario(requires(caps = ["draft/pre-away"]), covers = ["draft.pre-away"])]
async fn pre_away(s: Scenario) -> papo_testing::Result<()> {
    let away = "reviewing the overnight deployment";
    let mut alice = s
        .connect_with(s.configure("alice")?.away_on_connect(away))
        .await?;
    let bob = s.connect("bob").await?;

    let connected = alice.wait_for(Event::as_connected).await?;
    assert_eq!(connected.nick, alice.nick());
    assert!(connected.caps.contains_key("draft/pre-away"));
    assert!(alice.state().away());
    assert_eq!(alice.state().away_message(), None);

    let whois = bob.whois(alice.nick()).await?;
    assert_eq!(whois.nick, alice.nick());
    assert_eq!(whois.away.as_deref(), Some(away));
    alice.expect_none(Event::as_disconnected).await?;
    Ok(())
}

/// updates real-name state for the sender and every shared-channel peer
#[scenario(requires(caps = ["setname"]), covers = ["ircv3.setname"])]
async fn setname(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let channel = s.channel_name();
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
async fn chghost(s: Scenario) -> papo_testing::Result<()> {
    if std::env::var("PAPO_TEST_PROFILE").unwrap_or_else(|_| "ergo-default".into()) != "unrealircd"
    {
        return s.skip("the selected profile does not expose CHGHOST as an operator command");
    }
    let (oper_name, password) = oper_credentials(&s)?;
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let channel = s.channel_name();
    let bob_nick = bob.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    alice.oper(oper_name, password).await?;
    alice
        .send_raw(papo::Line::new(
            "CHGHOST",
            [bob_nick.as_ref(), "conformance.example"],
        ))
        .await?;
    let own = bob
        .wait_for(|event| {
            event
                .as_host_changed()
                .filter(|changed| changed.nick == bob_nick && changed.host == "conformance.example")
        })
        .await?;
    let observed = alice
        .wait_for(|event| {
            event
                .as_host_changed()
                .filter(|changed| changed.nick == bob_nick && changed.host == "conformance.example")
        })
        .await?;

    assert_eq!(own, observed);
    assert_eq!(own.nick, bob_nick);
    assert!(!own.username.is_empty());
    assert_eq!(own.host, "conformance.example");
    for peer in [&alice, &bob] {
        let state = peer.state();
        let user = state.user(&bob_nick).unwrap();
        assert_eq!(user.username(), Some(own.username.as_str()));
        assert_eq!(user.host(), Some("conformance.example"));
    }
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
async fn monitor(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let bob = s.connect("bob").await?;
    let bob_nick = bob.nick();
    alice.monitor().add([bob_nick.clone()]).await?;
    let online = alice.wait_for(Event::as_monitor_online).await?;

    assert_eq!(online.nick, bob_nick);
    let state = alice.state();
    let monitored = state.monitor_online().collect::<Vec<_>>();
    assert_eq!(monitored.len(), 1);
    assert_eq!(monitored[0].nick(), &bob_nick);
    assert_eq!(monitored[0].username(), online.username.as_deref());
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
async fn bot_mode(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect_with(s.configure("alice")?.bot()).await?;
    let mut bob = s.connect("bob").await?;
    let alice_nick = alice.nick();
    let channel = s.channel_name();
    let changed = alice
        .wait_for(|event| {
            event.as_mode_changed().filter(|changed| {
                subject_name(&changed.subject) == alice_nick.as_ref()
                    && changed.changes.iter().any(|change| change.mode == 'B')
            })
        })
        .await?;

    assert_eq!(subject_name(&changed.subject), alice_nick.as_ref());
    let bot_change = changed
        .changes
        .iter()
        .find(|change| change.mode == 'B')
        .unwrap();
    assert_eq!(bot_change.action, papo::LegacyModeAction::Add);
    assert!(alice.state().user_modes().any(|mode_| mode_ == 'B'));

    alice.join(&channel).await?;
    bob.join(&channel).await?;
    let whois = bob.whois(alice_nick.clone()).await?;

    assert_eq!(whois.nick, alice_nick);
    assert!(
        whois
            .username
            .as_ref()
            .is_some_and(|username| !username.is_empty())
    );
    assert!(whois.host.as_ref().is_some_and(|host| !host.is_empty()));
    assert!(whois.realname.as_ref().is_some_and(|name| !name.is_empty()));
    assert!(whois.is_bot);

    let text = "automated conformance report";
    alice.privmsg(&channel, text).await?;
    let message = bob
        .wait_for(|event| event.as_message().filter(|message| message.text() == text))
        .await?;

    assert_eq!(
        crate::source_nick(&message.actor),
        Some(alice_nick.as_ref())
    );
    assert_eq!(
        crate::scenarios::support::conversation_name(&message.conversation),
        Some(channel.as_str())
    );
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
async fn who_and_whois(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let bob = s.connect("bob").await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    let mapping = bob.state().casemapping();
    let channel_name = mapping.channel(&channel);
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    let away = "reading WHO reply fields";
    alice.set_away(away).await?;

    let who = bob.who(channel.clone()).await?;
    let alice_who = who.iter().find(|entry| entry.nick == alice_nick).unwrap();

    assert_eq!(alice_who.channel.as_ref(), Some(&channel_name));
    assert!(!alice_who.username.is_empty());
    assert!(!alice_who.host.is_empty());
    assert_eq!(alice_who.realname, "papo test peer");
    assert!(alice_who.away);
    assert!(!alice_who.is_oper);
    assert!(!alice_who.is_bot);
    assert_eq!(alice_who.prefixes, ['@']);

    // Plain WHO's numeric fields are the behavior under test, without WHOX rewriting them.
    let plain = bob.request_raw(papo::Line::new("WHO", [&channel])).await?;
    let row = plain
        .iter()
        .find(|line| {
            line.numeric() == Some(352)
                && line
                    .param(5)
                    .is_some_and(|nick| mapping.nick(nick) == alice_nick)
        })
        .unwrap();
    assert_eq!(
        mapping.channel(row.param(1).expect("352 includes the channel")),
        channel_name
    );
    assert!(row.param(2).is_some_and(|user| !user.is_empty()));
    assert!(row.param(3).is_some_and(|host| !host.is_empty()));
    assert!(row.param(4).is_some_and(|server| !server.is_empty()));
    assert!(row.param(6).is_some_and(|flags| flags.contains('G')));
    assert!(row.param(7).is_some_and(|hop_realname| {
        hop_realname
            .split_once(' ')
            .is_some_and(|(_, realname)| realname == "papo test peer")
    }));
    assert!(plain.iter().any(|line| {
        line.numeric() == Some(315)
            && line
                .param(1)
                .is_some_and(|name| mapping.channel(name) == channel_name)
    }));

    let whois = bob.whois(alice_nick.clone()).await?;

    assert_eq!(whois.nick, alice_nick);
    assert!(
        whois
            .username
            .as_ref()
            .is_some_and(|username| !username.is_empty())
    );
    assert!(whois.host.as_ref().is_some_and(|host| !host.is_empty()));
    assert_eq!(whois.realname.as_deref(), Some("papo test peer"));
    assert!(whois.idle.is_some());
    assert_eq!(whois.away.as_deref(), Some(away));
    assert!(
        whois
            .channels
            .iter()
            .any(|joined| joined.name == channel_name)
    );

    assert_refusal(
        bob.whois("missing-presence-target").await.unwrap_err(),
        Refusal::NoSuchNick,
        "401",
    );
    Ok(())
}

/// exposes operator status in WHO and WHOIS replies
#[scenario(covers = ["base.who.oper-flag", "base.whois.operator"])]
async fn operator_visibility(s: Scenario) -> papo_testing::Result<()> {
    let (oper_name, password) = oper_credentials(&s)?;
    let alice = s.connect("alice").await?;
    let bob = s.connect("bob").await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    let mapping = bob.state().casemapping();
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    alice.oper(oper_name, password).await?;
    assert!(alice.state().user_modes().any(|mode_| mode_ == 'o'));

    // The operator markers are encoded only in WHO flags and WHOIS numerics.
    let who = bob
        .request_raw(papo::Line::new("WHO", [alice_nick.as_ref()]))
        .await?;
    let row = who
        .iter()
        .find(|line| {
            line.numeric() == Some(352)
                && line
                    .param(5)
                    .is_some_and(|nick| mapping.nick(nick) == alice_nick)
        })
        .unwrap();
    assert!(row.param(6).is_some_and(|flags| flags.contains('*')));
    assert!(who.iter().any(|line| line.numeric() == Some(315)));

    let whois = bob
        .request_raw(papo::Line::new("WHOIS", [alice_nick.as_ref()]))
        .await?;
    let operator = whois
        .iter()
        .find(|line| line.numeric() == Some(313))
        .unwrap();
    assert_eq!(
        mapping.nick(
            operator
                .param(1)
                .expect("313 includes the operator nickname")
        ),
        alice_nick
    );
    assert!(operator.param(2).is_some_and(|text| !text.is_empty()));
    assert!(whois.iter().any(|line| line.numeric() == Some(318)));
    Ok(())
}

/// exercises operator authentication and forced disconnects
#[scenario(covers = [
    "base.error.message",
    "base.kill.relay",
    "base.oper.failure",
    "base.quit.server-initiated"
])]
async fn operator_commands(s: Scenario) -> papo_testing::Result<()> {
    let (oper_name, password) = oper_credentials(&s)?;
    let alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let mut charlie = s.connect("charlie").await?;
    let diana = s.connect("diana").await?;
    let channel = s.channel_name();
    let charlie_nick = charlie.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    charlie.join(&channel).await?;

    alice.oper(&oper_name, password).await?;
    let Error::Server(refusal) = diana
        .oper(&oper_name, "incorrect-password")
        .await
        .unwrap_err()
    else {
        panic!("incorrect operator credentials did not return a server refusal");
    };
    assert!(matches!(
        refusal.kind(),
        Refusal::PasswordIncorrect | Refusal::NoOperHost
    ));
    assert!(matches!(refusal.code(), "464" | "491"));

    alice
        .send_raw(papo::Line::new(
            "KILL",
            [charlie_nick.as_ref(), "conformance test"],
        ))
        .await?;
    let quit = bob
        .wait_for(|event| event.as_quit().filter(|quit| quit.nick == charlie_nick))
        .await?;
    assert_eq!(quit.nick, charlie_nick);
    assert!(
        quit.reason
            .as_deref()
            .is_some_and(|reason| !reason.is_empty())
    );
    let disconnected = charlie.wait_for(Event::as_disconnected).await?;
    let papo::DisconnectCause::Server { message } = disconnected.cause else {
        panic!("KILL did not produce a server ERROR disconnect");
    };
    assert!(message.contains("Killed"));
    Ok(())
}

/// relays operator broadcasts to users that request them
#[scenario(covers = ["base.wallops.relay"])]
async fn operator_wallops(s: Scenario) -> papo_testing::Result<()> {
    if std::env::var("PAPO_TEST_PROFILE").unwrap_or_else(|_| "ergo-default".into()) != "unrealircd"
    {
        return s.skip("the selected profile cannot combine a managed operator with user mode +w");
    }
    let (oper_name, password) = oper_credentials(&s)?;
    let alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    alice.oper(oper_name, password).await?;
    let bob_nick = bob.nick();
    bob.send_raw(papo::Line::new("MODE", [bob_nick.as_ref(), "+w"]))
        .await?;
    bob.wait_for(|event| {
        event.as_mode_changed().filter(|changed| {
            subject_name(&changed.subject) == bob_nick.as_ref()
                && changed.changes.iter().any(|change| {
                    change.mode == 'w' && change.action == papo::LegacyModeAction::Add
                })
        })
    })
    .await?;

    let wallops = "Network maintenance begins soon";
    alice
        .send_raw(papo::Line::new("WALLOPS", [wallops]))
        .await?;
    let delivered = bob
        .wait_for(|event| event.as_other().filter(|line| line.command() == "WALLOPS"))
        .await?;
    assert_eq!(delivered.params(), [wallops]);
    Ok(())
}

/// reports the target's server through WHOIS
#[scenario(covers = ["base.whois.server"])]
async fn whois_server(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let bob = s.connect("bob").await?;
    let alice_nick = alice.nick();

    let whois = bob.whois(alice_nick.clone()).await?;

    assert_eq!(whois.nick, alice_nick);
    if whois.server.as_ref().is_none_or(|server| server.is_empty()) {
        return Err(std::io::Error::other("WHOIS omitted server numeric 312").into());
    }
    Ok(())
}

/// reports a client's visible host or certificate fingerprint through WHOIS
#[scenario(covers = ["base.whois.host-cert"])]
async fn whois_host_certificate(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let alice_nick = alice.nick();
    let mapping = alice.state().casemapping();

    // The host and certificate numerics have no richer public result type.
    let lines = alice
        .request_raw(papo::Line::new("WHOIS", [alice_nick.as_ref()]))
        .await?;
    let detail = lines
        .iter()
        .find(|line| matches!(line.numeric(), Some(276 | 378)))
        .ok_or_else(|| {
            std::io::Error::other("WHOIS omitted host and certificate numerics 378 and 276")
        })?;

    assert_eq!(
        mapping.nick(detail.param(1).expect("WHOIS detail includes the nickname")),
        alice_nick
    );
    assert!(detail.param(2).is_some_and(|value| !value.is_empty()));
    assert!(lines.iter().any(|line| {
        line.numeric() == Some(318)
            && line
                .param(1)
                .is_some_and(|nick| mapping.nick(nick) == alice_nick)
    }));
    Ok(())
}

fn oper_credentials(s: &Scenario) -> papo_testing::Result<(String, String)> {
    if let Ok(oper) = std::env::var("PAPO_TEST_OPER") {
        let Some((name, password)) = oper.split_once(':') else {
            return Err("PAPO_TEST_OPER must contain name:password".into());
        };
        return Ok((name.to_owned(), password.to_owned()));
    }
    let profile = std::env::var("PAPO_TEST_PROFILE").unwrap_or_else(|_| "ergo-default".into());
    if matches!(
        profile.as_str(),
        "ergo-default" | "ergo-persistent" | "unrealircd"
    ) {
        return Ok((
            papo_test_server::OPER_NAME.into(),
            papo_test_server::OPER_PASSWORD.into(),
        ));
    }
    s.skip("the selected profile has no managed operator account")
}
