use papo::{
    Account, DisconnectCause, Error, Event, ModeAction, ModeArg, ModeValue, Rank, Refusal, Source,
    isupport::LimitValue,
};
use papo_conformance_macros::scenario;
use papo_test_server::DeploymentProfile;

use super::support::{assert_refusal, join_seen_by, subject_name, wait_channel_mode};
use crate::Scenario;

/// exposes the joining account and real name to every channel member
#[scenario(
    requires(caps = ["extended-join"]),
    covers = ["ircv3.extended-join"]
)]
async fn extended_join(s: Scenario) -> papo_testing::Result<()> {
    let mut bob = s.connect("bob").await?;
    let mut alice = s.connect_as_account("alice").await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    let account = crate::scenarios::support::logged_in_account(&alice.state()).clone();
    bob.join(&channel).await?;
    let joined = bob.wait_for(Event::as_joined).await?;
    assert_eq!(joined.channel.as_ref(), channel);
    assert_eq!(joined.nick, bob_nick);
    assert_eq!(joined.authorship, papo::Authorship::Own);

    alice.join(&channel).await?;
    let joined = alice.wait_for(Event::as_joined).await?;
    assert_eq!(joined.channel.as_ref(), channel);
    assert_eq!(joined.nick, alice_nick);
    assert_eq!(joined.authorship, papo::Authorship::Own);
    let joined = bob.wait_for(Event::as_joined).await?;

    assert_eq!(joined.channel.as_ref(), channel);
    assert_eq!(joined.nick, alice_nick);
    assert_eq!(joined.authorship, papo::Authorship::Other);
    let bob_state = bob.state();
    let observed = bob_state.user(&alice_nick).unwrap();
    assert_eq!(observed.realname(), Some("papo test peer"));
    assert!(matches!(observed.account(), Account::LoggedIn(name) if name == &account));
    assert!(
        bob_state
            .channel(&channel)
            .unwrap()
            .member(&alice_nick)
            .is_some()
    );
    assert!(
        alice
            .state()
            .channel(&channel)
            .unwrap()
            .member(&bob_nick)
            .is_some()
    );
    bob.expect_none(Event::as_joined).await?;
    Ok(())
}

/// records complete userhosts from the initial names reply
#[scenario(
    requires(caps = ["userhost-in-names"]),
    covers = ["base.names.basic", "ircv3.userhost-in-names"]
)]
async fn userhost_in_names(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let bob = s.connect("bob").await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    alice.join(&channel).await?;

    bob.join(&channel).await?;
    let joined = alice
        .wait_for(|event| event.as_joined().filter(|joined| joined.nick == bob_nick))
        .await?;
    assert_eq!(joined.channel.as_ref(), channel);

    let bob_state = bob.state();
    let observed = bob_state.user(&alice_nick).unwrap();
    assert!(observed.username().is_some_and(|user| !user.is_empty()));
    assert!(observed.host().is_some_and(|host| !host.is_empty()));
    assert!(
        bob_state
            .channel(&channel)
            .unwrap()
            .member(&alice_nick)
            .is_some()
    );
    assert!(
        alice
            .state()
            .channel(&channel)
            .unwrap()
            .member(&bob_nick)
            .is_some()
    );
    Ok(())
}

/// returns names and completion for every channel in one request
#[scenario(covers = ["base.names.multiple-channels"])]
async fn names_multiple_channels(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let first = s.channel_name();
    let second = s.channel_name();
    alice.join(&first).await?;
    alice.join(&second).await?;

    // NAMES numerics and their channel parameters are the behavior under test.
    let targets = format!("{first},{second}");
    let mapping = alice.state().casemapping();
    let names = alice
        .request_raw(papo::Line::new("NAMES", [&targets]))
        .await?;
    for channel in [&first, &second] {
        let channel_name = mapping.channel(channel);
        if !names.iter().any(|line| {
            line.numeric() == Some(353)
                && line
                    .param(2)
                    .is_some_and(|name| mapping.channel(name) == channel_name)
        }) {
            return Err(std::io::Error::other(format!(
                "missing names row for {channel}: {names:?}"
            ))
            .into());
        }
        assert!(
            names.iter().any(|line| {
                line.numeric() == Some(366)
                    && line.param(1).is_some_and(|targets| {
                        targets
                            .split(',')
                            .any(|target| mapping.channel(target) == channel_name)
                    })
            }),
            "missing names terminator for {channel}: {names:?}"
        );
    }
    assert!(alice.state().channel(&first).is_some());
    assert!(alice.state().channel(&second).is_some());
    Ok(())
}

/// hides the names of a secret channel from clients outside it
#[scenario(covers = ["base.names.secret-channel-visibility"])]
async fn secret_names_visibility(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let bob = s.connect("bob").await?;
    let channel = s.channel_name();
    let mapping = bob.state().casemapping();
    let channel_name = mapping.channel(&channel);
    alice.join(&channel).await?;
    alice.mode(&channel).add('s').await?;

    // The absence of a names row in this numeric response proves the visibility rule.
    let hidden = bob
        .request_raw(papo::Line::new("NAMES", [&channel]))
        .await?;
    assert!(
        !hidden.iter().any(|line| {
            line.numeric() == Some(353)
                && line
                    .param(2)
                    .is_some_and(|name| mapping.channel(name) == channel_name)
        }),
        "secret channel leaked through NAMES: {hidden:?}"
    );
    assert!(
        hidden.iter().any(|line| {
            line.numeric() == Some(366)
                && line
                    .param(1)
                    .is_some_and(|name| mapping.channel(name) == channel_name)
        }),
        "missing hidden-channel terminator: {hidden:?}"
    );
    assert!(alice.state().channel(&channel).is_some());
    assert!(bob.state().channel(&channel).is_none());
    Ok(())
}

/// synchronizes a join when the server suppresses its implicit names burst
#[scenario(
    requires(caps = ["no-implicit-names"]),
    covers = ["ircv3.no-implicit-names"]
)]
async fn no_implicit_names(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    assert!(
        alice
            .state()
            .caps()
            .contains_key(&papo::cap::NO_IMPLICIT_NAMES)
    );

    alice.join(&channel).await?;
    let joined = alice.wait_for(Event::as_joined).await?;
    let synced = alice.wait_for(Event::as_channel_synced).await?;
    assert_eq!(joined.channel.as_ref(), channel);
    assert_eq!(joined.nick, alice_nick);
    assert_eq!(joined.authorship, papo::Authorship::Own);
    assert_eq!(synced.channel.as_ref(), channel);
    let state = alice.state();
    let joined_channel = state.channel(&channel).unwrap();
    assert!(joined_channel.is_synced());
    assert!(joined_channel.member(&alice_nick).is_some());
    alice.expect_none(Event::as_channel_synced).await?;
    Ok(())
}

/// joins several keyed channels and leaves all of them with JOIN zero
#[scenario(covers = [
    "base.join.basic",
    "base.join.multiple-channels",
    "base.join.key-multiple",
    "base.join.zero-part-all"
])]
async fn multiple_joins(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let first = s.channel_name();
    let second = s.channel_name();
    let bob_nick = bob.nick();
    let channels = format!("{first},{second}");
    let keys = "first-channel-key,second-channel-key";
    alice.join(&first).await?;
    alice.join(&second).await?;
    alice
        .mode(&first)
        .add_with('k', "first-channel-key")
        .await?;
    alice
        .mode(&second)
        .add_with('k', "second-channel-key")
        .await?;

    bob.send_raw(papo::Line::new("JOIN", [&channels, keys]))
        .await?;
    for channel in [&first, &second] {
        let synced = bob
            .wait_for(|event| {
                event
                    .as_channel_synced()
                    .filter(|synced| synced.channel.as_ref() == channel)
            })
            .await?;
        assert_eq!(synced.channel.as_ref(), channel);
        let joined = alice
            .wait_for(|event| {
                event
                    .as_joined()
                    .filter(|joined| joined.channel.as_ref() == channel && joined.nick == bob_nick)
            })
            .await?;
        assert_eq!(joined.authorship, papo::Authorship::Other);
        assert!(bob.state().channel(channel).is_some());
        assert!(
            alice
                .state()
                .channel(channel)
                .unwrap()
                .member(&bob_nick)
                .is_some()
        );
    }

    bob.send_raw(papo::Line::new("JOIN", ["0"])).await?;
    if matches!(
        DeploymentProfile::from_env()?,
        DeploymentProfile::ErgoDefault | DeploymentProfile::ErgoPersistent
    ) {
        let warning = bob
            .wait_for(|event| {
                event.as_message().filter(|message| {
                    message
                        .text
                        .starts_with("Warning: /JOIN 0 will remove you from all channels")
                })
            })
            .await?;
        let confirmation = warning.text.split_ascii_whitespace().last().unwrap();
        bob.send_raw(papo::Line::new("JOIN", ["0", confirmation]))
            .await?;
    }
    let mut own = [
        bob.wait_for(Event::as_parted).await?,
        bob.wait_for(Event::as_parted).await?,
    ];
    let mut observed = [
        alice.wait_for(Event::as_parted).await?,
        alice.wait_for(Event::as_parted).await?,
    ];
    own.sort_unstable_by(|left, right| left.channel.cmp(&right.channel));
    observed.sort_unstable_by(|left, right| left.channel.cmp(&right.channel));
    let mut channels = [&first, &second];
    channels.sort_unstable();
    for ((own, observed), channel) in own.iter().zip(&observed).zip(channels) {
        assert_eq!(own.channel.as_ref(), channel);
        assert_eq!(observed.channel.as_ref(), channel);
        assert_eq!(own.nick, bob_nick);
        assert_eq!(observed.nick, bob_nick);
        assert_eq!(own.authorship, papo::Authorship::Own);
        assert_eq!(observed.authorship, papo::Authorship::Other);
        assert!(bob.state().channel(channel).is_none());
        assert!(
            alice
                .state()
                .channel(channel)
                .unwrap()
                .member(&bob_nick)
                .is_none()
        );
    }
    Ok(())
}

/// accepts valid JOIN targets while reporting a malformed channel mask
#[scenario(covers = [
    "base.join.err-bad-chanmask",
    "base.join.partially-invalid"
])]
async fn partially_invalid_join(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let channel = s.channel_name();
    let invalid = format!("#invalid\x07{}", channel.trim_start_matches('#'));

    // A single JOIN command must still apply its valid target before reporting the bad one.
    let targets = format!("{channel},{invalid}");
    let partial = alice
        .request_raw(papo::Line::new("JOIN", [&targets]))
        .await?;
    let synced = alice
        .wait_for(|event| {
            event
                .as_channel_synced()
                .filter(|synced| synced.channel.as_ref() == channel)
        })
        .await?;
    assert_eq!(synced.channel.as_ref(), channel);
    let state = alice.state();
    assert!(
        state
            .channel(&channel)
            .is_some_and(papo::Channel::is_synced)
    );
    assert!(state.channel(&invalid).is_none());
    if !partial.iter().any(|line| line.numeric() == Some(476)) {
        return Err(std::io::Error::other(format!(
            "unexpected malformed JOIN response: {partial:?}"
        ))
        .into());
    }
    Ok(())
}

/// refuses the first JOIN target beyond the advertised channel limit
#[scenario(covers = ["base.join.err-too-many-channels"])]
async fn too_many_channels(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let Some(LimitValue::Limited(limit)) = alice
        .state()
        .isupport()
        .chanlimit()
        .into_iter()
        .find(|entry| entry.name.contains('#'))
        .and_then(|entry| entry.limit)
    else {
        return s.skip("server does not advertise a finite # channel limit");
    };
    if limit > 15 {
        return s.skip(format!(
            "advertised channel limit {limit} exceeds bounded live-test capacity"
        ));
    }
    let channels = (0..=limit).map(|_| s.channel_name()).collect::<Vec<_>>();
    let targets = channels.join(",");
    let mapping = alice.state().casemapping();

    // The server numeric is the behavior under test, so retain its raw response.
    alice.send_raw(papo::Line::new("JOIN", [&targets])).await?;
    let refusal = alice
        .wait_for(|event| event.as_other().filter(|line| line.command() == "405"))
        .await?;
    assert_eq!(refusal.numeric(), Some(405));
    assert_eq!(
        mapping.channel(refusal.param(1).expect("405 includes the refused channel")),
        channels[limit].as_str()
    );
    assert_eq!(alice.state().channels().count(), limit);
    assert!(alice.state().channel(&channels[limit]).is_none());
    Ok(())
}

/// returns a complete identity from an extended WHO query
#[scenario(
    requires(isupport = ["WHOX"]),
    covers = ["base.who.whox-extended", "ircv3.whox"]
)]
async fn whox(s: Scenario) -> papo_testing::Result<()> {
    let alice = s
        .connect_with(s.configure("alice")?.realname("Alice Example"))
        .await?;
    let nick = alice.nick();

    let entries = alice.who(nick.as_ref()).await?;
    let entry = entries
        .iter()
        .find(|entry| entry.nick == nick)
        .expect("WHO must include its exact target");

    assert_eq!(entry.nick, nick);
    assert!(!entry.username.is_empty());
    assert!(!entry.host.is_empty());
    assert_eq!(entry.realname, "Alice Example");
    assert_eq!(entry.away, papo::Away::Present);
    assert!(!entry.is_oper);
    Ok(())
}

/// preserves every membership prefix in the joined member list
#[scenario(
    requires(caps = ["multi-prefix"]),
    covers = [
        "base.isupport.prefix",
        "base.mode.channel.op-voice-prefix",
        "ircv3.multi-prefix"
    ]
)]
async fn multi_prefix(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    alice.join(&channel).await?;

    alice.mode(&channel).voice(alice_nick.clone()).await?;
    let changed = wait_channel_mode(&mut alice, &channel, 'v').await?;

    assert_eq!(subject_name(&changed.subject), channel);
    assert_actor_or_server(&changed.actor, alice_nick.as_ref());
    let voice = changed
        .changes
        .iter()
        .find(|change| change.mode == 'v')
        .unwrap();
    assert_eq!(voice.action, ModeAction::Add);
    assert!(matches!(voice.arg.clone(), Some(ModeArg::Nick(nick)) if nick == alice_nick));
    bob.join(&channel).await?;
    let joined = alice.wait_for(Event::as_joined).await?;
    assert_eq!(joined.channel.as_ref(), channel);
    assert_eq!(joined.nick, bob_nick);
    let prefixes = bob
        .state()
        .channel(&channel)
        .and_then(|channel| channel.member(&alice_nick))
        .map(|member| member.prefixes().to_vec());
    assert_eq!(prefixes.as_deref(), Some(&['@', '+'][..]));
    assert!(
        alice
            .state()
            .channel(&channel)
            .unwrap()
            .member(&bob_nick)
            .is_some()
    );
    bob.expect_none(|event| {
        event
            .as_mode_changed()
            .filter(|changed| subject_name(&changed.subject) == channel)
    })
    .await?;
    Ok(())
}

/// delivers an invitation to its target and every capable channel observer
#[scenario(
    requires(caps = ["invite-notify"]),
    covers = [
        "base.invite.sender-ack",
        "base.invite.target-delivery",
        "base.invite.observer-notify",
        "ircv3.invite-notify"
    ]
)]
async fn invite_notify(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let mut carol = s.connect("carol").await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    let carol_nick = carol.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    alice.mode(&channel).op(bob_nick.clone()).await?;
    let promoted = wait_channel_mode(&mut bob, &channel, 'o').await?;

    assert_eq!(subject_name(&promoted.subject), channel);
    assert_actor_or_server(&promoted.actor, alice_nick.as_ref());
    let operator = promoted
        .changes
        .iter()
        .find(|change| change.mode == 'o')
        .unwrap();
    assert_eq!(operator.action, ModeAction::Add);
    assert!(matches!(operator.arg.clone(), Some(ModeArg::Nick(nick)) if nick == bob_nick));
    assert!(
        bob.state()
            .channel(&channel)
            .unwrap()
            .member(&bob_nick)
            .unwrap()
            .rank()
            >= Some(Rank::Op)
    );

    alice.invite(&channel, carol_nick.clone()).await?;
    let target = carol.wait_for(Event::as_invited).await?;
    let observer = bob.wait_for(Event::as_invited).await?;

    assert_eq!(target.channel.as_ref(), channel);
    assert_eq!(target.nick.as_ref(), carol_nick.as_ref());
    assert_eq!(crate::source_nick(&target.actor), Some(alice_nick.as_ref()));
    assert_eq!(target.subject_authorship, papo::Authorship::Own);
    assert_eq!(observer.channel.as_ref(), channel);
    assert_eq!(observer.nick.as_ref(), carol_nick.as_ref());
    assert_eq!(
        crate::source_nick(&observer.actor),
        Some(alice_nick.as_ref())
    );
    assert_eq!(observer.subject_authorship, papo::Authorship::Other);
    assert!(
        alice
            .state()
            .channel(&channel)
            .unwrap()
            .member(&carol_nick)
            .is_none()
    );
    assert!(
        bob.state()
            .channel(&channel)
            .unwrap()
            .member(&carol_nick)
            .is_none()
    );
    carol.expect_none(Event::as_invited).await?;
    Ok(())
}

/// enforces invitation privileges and lets an invitation override a channel ban
#[scenario(covers = [
    "base.invite.err-not-op",
    "base.invite.err-already-on-channel",
    "base.invite.exempts-ban"
])]
async fn invite_rules(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let bob = s.connect("bob").await?;
    let mut carol = s.connect("carol").await?;
    let channel = s.channel_name();
    let bob_nick = bob.nick();
    let carol_nick = carol.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    alice
        .mode(&channel)
        .ban(
            alice
                .state()
                .casemapping()
                .mask(format!("{carol_nick}!*@*")),
        )
        .add('i')
        .await?;

    assert_refusal(
        bob.invite(&channel, carol_nick.clone()).await.unwrap_err(),
        Refusal::ChannelOperatorPrivilegesNeeded,
        "482",
    );
    assert_refusal(
        alice.invite(&channel, bob_nick).await.unwrap_err(),
        Refusal::UserOnChannel,
        "443",
    );

    alice.invite(&channel, carol_nick.clone()).await?;
    let invited = carol.wait_for(Event::as_invited).await?;
    assert_eq!(invited.channel.as_ref(), channel);
    assert_eq!(invited.nick, carol_nick);
    assert_eq!(invited.subject_authorship, papo::Authorship::Own);
    carol.join(&channel).await?;
    let joined = alice
        .wait_for(|event| event.as_joined().filter(|joined| joined.nick == carol_nick))
        .await?;
    assert_eq!(joined.channel.as_ref(), channel);
    assert_eq!(joined.authorship, papo::Authorship::Other);
    assert!(
        alice
            .state()
            .channel(&channel)
            .unwrap()
            .member(&carol_nick)
            .is_some()
    );
    assert!(
        carol
            .state()
            .channel(&channel)
            .unwrap()
            .member(&carol_nick)
            .is_some()
    );
    Ok(())
}

/// removes a departing member from both peers with the same reason
#[scenario(covers = ["base.part.basic"])]
async fn part(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    let reason = "leaving for the specification review";
    alice.part(&channel, reason).await?;
    let own = alice.wait_for(Event::as_parted).await?;
    let observed = bob.wait_for(Event::as_parted).await?;

    for departed in [&own, &observed] {
        assert_eq!(departed.channel.as_ref(), channel);
        assert_eq!(departed.nick, alice_nick);
        assert_eq!(departed.reason.as_deref(), Some(reason));
    }
    assert_eq!(own.authorship, papo::Authorship::Own);
    assert_eq!(observed.authorship, papo::Authorship::Other);
    assert!(alice.state().channel(&channel).is_none());
    assert!(
        bob.state()
            .channel(&channel)
            .unwrap()
            .member(&alice_nick)
            .is_none()
    );
    alice.expect_none(Event::as_parted).await?;
    bob.expect_none(Event::as_parted).await?;
    Ok(())
}

/// parts several channels together and rejects a channel the actor has not joined
#[scenario(covers = [
    "base.part.multiple-channels",
    "base.part.err-not-on-channel"
])]
async fn part_multiple_and_refusal(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let first = s.channel_name();
    let second = s.channel_name();
    let alice_nick = alice.nick();
    for channel in [&first, &second] {
        alice.join(channel).await?;
        bob.join(channel).await?;
    }

    let channels = format!("{first},{second}");
    let reason = "leaving both review channels";
    alice
        .send_raw(papo::Line::new("PART", [&channels, reason]))
        .await?;
    let mut own = [
        alice.wait_for(Event::as_parted).await?,
        alice.wait_for(Event::as_parted).await?,
    ];
    let mut observed = [
        bob.wait_for(Event::as_parted).await?,
        bob.wait_for(Event::as_parted).await?,
    ];
    own.sort_unstable_by(|left, right| left.channel.cmp(&right.channel));
    observed.sort_unstable_by(|left, right| left.channel.cmp(&right.channel));
    let mut expected = [&first, &second];
    expected.sort_unstable();
    for ((own, observed), channel) in own.iter().zip(&observed).zip(expected) {
        assert_eq!(own.channel.as_ref(), channel);
        assert_eq!(observed.channel.as_ref(), channel);
        assert_eq!(own.nick, alice_nick);
        assert_eq!(observed.nick, alice_nick);
        assert_eq!(own.reason.as_deref(), Some(reason));
        assert_eq!(observed.reason.as_deref(), Some(reason));
        assert_eq!(own.authorship, papo::Authorship::Own);
        assert_eq!(observed.authorship, papo::Authorship::Other);
        assert!(alice.state().channel(channel).is_none());
        assert!(
            bob.state()
                .channel(channel)
                .unwrap()
                .member(&alice_nick)
                .is_none()
        );
    }

    assert_refusal(
        alice.part(&first, "not joined here").await.unwrap_err(),
        Refusal::NotOnChannel,
        "442",
    );
    alice.expect_none(Event::as_parted).await?;
    Ok(())
}

/// disconnects cleanly and removes the user from every shared channel
#[scenario(covers = ["base.quit.self", "base.quit.peer-broadcast"])]
async fn quit(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    let reason = "conformance review complete";

    alice.quit(reason).await?;
    let disconnected = alice.wait_for(Event::as_disconnected).await?;
    let observed = bob.wait_for(Event::as_quit).await?;

    assert_eq!(
        disconnected.cause,
        DisconnectCause::Requested {
            quit_unacknowledged: false,
        }
    );
    assert_eq!(disconnected.reconnect_in, None);
    assert_eq!(observed.nick, alice_nick);
    let observed_reason = observed.reason.as_deref().unwrap();
    assert!(observed_reason == reason || observed_reason == format!("Quit: {reason}"));
    assert_eq!(observed.channels.len(), 1);
    assert_eq!(observed.channels[0].as_ref(), channel);
    assert!(
        bob.state()
            .channel(&channel)
            .unwrap()
            .member(&alice_nick)
            .is_none()
    );
    bob.expect_none(Event::as_quit).await?;
    Ok(())
}

/// removes a kicked member from the channel state of the target and operator
#[scenario(covers = ["base.kick.basic"])]
async fn kick(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    let reason = "refresh channel membership";
    alice.kick(&channel, bob_nick.clone(), reason).await?;
    let observed = alice.wait_for(Event::as_kicked).await?;
    let own = bob.wait_for(Event::as_kicked).await?;

    for kicked in [&observed, &own] {
        assert_eq!(kicked.channel.as_ref(), channel);
        assert_eq!(kicked.nick, bob_nick);
        assert_eq!(crate::source_nick(&kicked.actor), Some(alice_nick.as_ref()));
        assert_eq!(kicked.reason.as_deref(), Some(reason));
    }
    assert_eq!(observed.subject_authorship, papo::Authorship::Other);
    assert_eq!(own.subject_authorship, papo::Authorship::Own);
    assert!(
        alice
            .state()
            .channel(&channel)
            .unwrap()
            .member(&bob_nick)
            .is_none()
    );
    assert!(bob.state().channel(&channel).is_none());
    alice.expect_none(Event::as_kicked).await?;
    bob.expect_none(Event::as_kicked).await?;
    Ok(())
}

/// confirms a ban before kicking both a present and rejoined member
#[scenario(covers = ["papo.moderation.kickban-confirmed"])]
async fn kickban(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let channel = s.channel_name();
    let bob_nick = bob.nick();
    let ban = format!("repeat-{bob_nick}!*@gateway.example");
    alice.join(&channel).await?;
    join_seen_by(&bob, &mut alice, &channel).await?;

    let first = alice
        .kickban(&channel, &bob_nick, "repeated flooding", ban.as_str())
        .await?;
    first.kick?;
    let kicked = bob.wait_for(Event::as_kicked).await?;
    assert_eq!(kicked.nick, bob_nick);

    join_seen_by(&bob, &mut alice, &channel).await?;
    let duplicate = alice
        .kickban(&channel, &bob_nick, "ban already present", ban.as_str())
        .await?;
    duplicate.kick?;
    let kicked = bob.wait_for(Event::as_kicked).await?;
    assert_eq!(kicked.nick, bob_nick);
    assert_eq!(kicked.reason.as_deref(), Some("ban already present"));
    assert!(
        alice
            .list_modes(&channel, papo::ListMode::Ban)
            .await?
            .iter()
            .any(|entry| alice.state().same_list_entry(&entry.mask, &ban))
    );
    alice.mode(&channel).unban(ban).await?;
    Ok(())
}

/// rejects unauthorized kicks and applies a default comment to multiple targets
#[scenario(covers = [
    "base.kick.multi-target-split",
    "base.kick.err-not-privileged",
    "base.kick.default-comment"
])]
async fn kick_rules(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let mut carol = s.connect("carol").await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    let carol_nick = carol.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    carol.join(&channel).await?;

    assert_refusal(
        bob.kick(&channel, carol_nick.clone(), "not permitted")
            .await
            .unwrap_err(),
        Refusal::ChannelOperatorPrivilegesNeeded,
        "482",
    );
    let targets = format!("{bob_nick},{carol_nick}");
    alice
        .send_raw(papo::Line::new("KICK", [&channel, &targets]))
        .await?;
    let mut observed = [
        alice.wait_for(Event::as_kicked).await?,
        alice.wait_for(Event::as_kicked).await?,
    ];
    observed.sort_unstable_by(|left, right| left.nick.cmp(&right.nick));
    let mut own = [
        bob.wait_for(|event| event.as_kicked().filter(|kicked| kicked.nick == bob_nick))
            .await?,
        carol
            .wait_for(|event| event.as_kicked().filter(|kicked| kicked.nick == carol_nick))
            .await?,
    ];
    own.sort_unstable_by(|left, right| left.nick.cmp(&right.nick));
    let mut expected = [&bob_nick, &carol_nick];
    expected.sort_unstable();
    for ((observed, own), nick) in observed.iter().zip(&own).zip(expected) {
        assert_eq!(&observed.nick, nick);
        assert_eq!(&own.nick, nick);
        assert_eq!(
            crate::source_nick(&observed.actor),
            Some(alice_nick.as_ref())
        );
        assert_eq!(crate::source_nick(&own.actor), Some(alice_nick.as_ref()));
        assert_eq!(observed.reason.as_deref(), Some(alice_nick.as_ref()));
        assert_eq!(own.reason, observed.reason);
        assert_eq!(observed.subject_authorship, papo::Authorship::Other);
        assert_eq!(own.subject_authorship, papo::Authorship::Own);
        assert!(
            alice
                .state()
                .channel(&channel)
                .unwrap()
                .member(nick)
                .is_none()
        );
    }
    assert!(bob.state().channel(&channel).is_none());
    assert!(carol.state().channel(&channel).is_none());
    Ok(())
}

/// rejects an operator's attempt to kick a user outside the channel
#[scenario(covers = ["base.kick.err-user-not-in-channel"])]
async fn kick_missing_member(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let channel = s.channel_name();
    let bob_nick = bob.nick();
    alice.join(&channel).await?;

    let refusal = match alice
        .kick(&channel, bob_nick, "not a channel member")
        .await
        .unwrap_err()
    {
        Error::Server(refusal) => refusal,
        other => return Err(other.into()),
    };
    if refusal.refusal() != Refusal::UserNotInChannel || refusal.code() != "441" {
        return Err(std::io::Error::other(format!(
            "unexpected missing-member refusal: {refusal:?}"
        ))
        .into());
    }
    assert_eq!(alice.state().channels().count(), 1);
    assert_eq!(bob.state().channels().count(), 0);
    alice.expect_none(Event::as_kicked).await?;
    bob.expect_none(Event::as_kicked).await?;
    Ok(())
}

/// propagates a nickname change and rejects a later claim of the same nickname
#[scenario(covers = ["base.nick.change", "base.nick.collision-in-use"])]
async fn nick_change_and_collision(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let channel = s.channel_name();
    let old_nick = alice.nick();
    let bob_nick = bob.nick();
    let new_nick = format!("r{old_nick}");
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    alice.set_nick(new_nick.clone()).await?;
    let own = alice.wait_for(Event::as_nick_changed).await?;
    let observed = bob.wait_for(Event::as_nick_changed).await?;

    for changed in [&own, &observed] {
        assert_eq!(changed.old, old_nick);
        assert_eq!(changed.new.as_ref(), new_nick);
    }
    assert_eq!(own.authorship, papo::Authorship::Own);
    assert_eq!(observed.authorship, papo::Authorship::Other);
    assert_eq!(alice.state().nick(), new_nick.as_str());
    assert!(
        alice
            .state()
            .channel(&channel)
            .unwrap()
            .member(&new_nick)
            .is_some()
    );
    assert!(
        bob.state()
            .channel(&channel)
            .unwrap()
            .member(&new_nick)
            .is_some()
    );
    assert!(
        bob.state()
            .channel(&channel)
            .unwrap()
            .member(&old_nick)
            .is_none()
    );

    assert_refusal(
        bob.set_nick(new_nick.clone()).await.unwrap_err(),
        Refusal::NicknameInUse,
        "433",
    );
    assert_eq!(bob.state().nick(), &bob_nick);
    bob.expect_none(Event::as_nick_changed).await?;
    Ok(())
}

/// rejects empty and malformed nickname changes without changing local state
#[scenario(covers = ["base.nick.empty", "base.nick.invalid-chars"])]
async fn nick_errors(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let original = alice.nick();

    // Empty and malformed wire parameters are needed to observe 431 and 432.
    let empty = alice.request_raw(papo::Line::new("NICK", [""])).await?;
    assert!(
        empty.iter().any(|line| line.numeric() == Some(431)),
        "unexpected empty-nickname response: {empty:?}"
    );
    let malformed = alice
        .request_raw(papo::Line::new("NICK", ["review#nick"]))
        .await?;
    assert!(
        malformed.iter().any(|line| line.numeric() == Some(432)),
        "unexpected malformed-nickname response: {malformed:?}"
    );
    assert_eq!(alice.state().nick(), &original);
    alice.expect_none(Event::as_nick_changed).await?;
    Ok(())
}

/// races two connected clients for one unclaimed nickname
#[scenario(covers = ["base.nick.collision-race"])]
async fn nickname_collision_race(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let bob = s.connect("bob").await?;
    let claimed = format!("race-{}", alice.nick());
    let alice_client = papo::Client::clone(&alice);
    let bob_client = papo::Client::clone(&bob);
    let mapping = alice.state().casemapping();
    let alice_claim = claimed.clone();
    let bob_claim = claimed.clone();

    let alice_result =
        tokio::task::spawn(async move { alice_client.set_nick(mapping.nick(alice_claim)).await });
    let bob_result =
        tokio::task::spawn(async move { bob_client.set_nick(mapping.nick(bob_claim)).await });
    let results = [alice_result.await?, bob_result.await?];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let error = results
        .into_iter()
        .find_map(Result::err)
        .expect("one racing nickname claim must be refused");
    let refusal = match error {
        Error::Server(refusal) => refusal,
        other => panic!("nickname race did not return a server refusal: {other:?}"),
    };
    if refusal.refusal() != Refusal::NickCollision || refusal.code() != "436" {
        return Err(std::io::Error::other(format!(
            "unexpected nickname-race refusal: {refusal:?}"
        ))
        .into());
    }
    assert!(alice.state().nick() == claimed.as_str() || bob.state().nick() == claimed.as_str());
    Ok(())
}

/// synchronizes topic metadata and propagates setting and clearing the topic
#[scenario(covers = [
    "base.topic.get",
    "base.topic.set",
    "base.topic.clear",
    "base.topic.whotime"
])]
async fn topic(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let mut carol = s.connect("carol").await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    let text = "Reviewing IRC topic synchronization";
    alice.set_topic(&channel, text).await?;
    let own = alice.wait_for(Event::as_topic_changed).await?;
    let observed = bob.wait_for(Event::as_topic_changed).await?;

    for changed in [&own, &observed] {
        assert_eq!(changed.channel.as_ref(), channel);
        assert_eq!(changed.topic.as_deref(), Some(text));
        assert_eq!(
            crate::source_nick(&changed.actor),
            Some(alice_nick.as_ref())
        );
    }
    for peer in [&alice, &bob] {
        assert_eq!(peer.state().channel(&channel).unwrap().topic(), Some(text));
    }

    carol.join(&channel).await?;
    let state = carol.state();
    let synchronized = state.channel(&channel).unwrap();
    assert_eq!(synchronized.topic(), Some(text));
    assert_eq!(
        synchronized.topic_set_by().and_then(crate::source_nick),
        Some(alice_nick.as_ref())
    );
    assert!(synchronized.topic_set_at().is_some());

    alice
        .clear_topic(alice.state().casemapping().channel(&channel))
        .await?;
    for peer in [&mut alice, &mut bob, &mut carol] {
        let changed = peer.wait_for(Event::as_topic_changed).await?;
        assert_eq!(changed.channel.as_ref(), channel);
        assert_eq!(changed.topic, None);
        assert_eq!(
            crate::source_nick(&changed.actor),
            Some(alice_nick.as_ref())
        );
        assert_eq!(peer.state().channel(&channel).unwrap().topic(), None);
        peer.expect_none(Event::as_topic_changed).await?;
    }
    Ok(())
}

/// rejects a topic change from a member without channel privileges
#[scenario(covers = ["base.topic.protected-mode"])]
async fn protected_topic(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let channel = s.channel_name();
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    assert_refusal(
        bob.set_topic(&channel, "unauthorized topic")
            .await
            .unwrap_err(),
        Refusal::ChannelOperatorPrivilegesNeeded,
        "482",
    );
    assert_eq!(alice.state().channel(&channel).unwrap().topic(), None);
    assert_eq!(bob.state().channel(&channel).unwrap().topic(), None);
    alice.expect_none(Event::as_topic_changed).await?;
    bob.expect_none(Event::as_topic_changed).await?;
    Ok(())
}

/// reports channel modes and round-trips the ban list
#[scenario(covers = [
    "base.mode.channel.query",
    "base.mode.channel.ban",
    "papo.moderation.list-modes"
])]
async fn channel_mode_lists(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let channel = s.channel_name();
    let mapping = alice.state().casemapping();
    let channel_name = mapping.channel(&channel);
    let mask = mapping.mask("troublesome!*@gateway.example");
    alice.join(&channel).await?;
    alice.mode(&channel).ban(&mask).await?;

    // MODE query numerics expose list entries that are not stored as channel flags.
    let modes = alice
        .request_raw(papo::Line::new("MODE", [&channel]))
        .await?;
    let current = modes
        .iter()
        .find(|line| line.numeric() == Some(324))
        .expect("channel mode query must include 324");
    assert_eq!(
        mapping.channel(current.param(1).expect("324 includes the channel")),
        channel_name
    );
    assert!(current.param(2).is_some_and(|value| value.starts_with('+')));

    let bans = alice.list_modes(&channel, papo::ListMode::Ban).await?;
    let entry = bans.iter().find(|entry| entry.mask == mask).unwrap();
    assert!(entry.actor.is_some());
    assert!(entry.time.is_some());
    let tracked = alice
        .state()
        .channel(&channel)
        .unwrap()
        .list(papo::ListMode::Ban)
        .map(<[_]>::len);
    if DeploymentProfile::from_env()? == DeploymentProfile::SojuErgo {
        assert_eq!(tracked, None);
    } else {
        assert_eq!(tracked, Some(1));
    }

    alice.mode(&channel).unban(&mask).await?;
    let cleared = alice.list_modes(&channel, papo::ListMode::Ban).await?;
    assert!(cleared.is_empty());
    let tracked = alice
        .state()
        .channel(&channel)
        .unwrap()
        .list(papo::ListMode::Ban)
        .map(<[_]>::len);
    if DeploymentProfile::from_env()? == DeploymentProfile::SojuErgo {
        assert_eq!(tracked, None);
    } else {
        assert_eq!(tracked, Some(0));
    }
    Ok(())
}

/// round-trips advertised ban and invitation exception lists
#[scenario(
    requires(isupport = ["EXCEPTS", "INVEX"]),
    covers = [
        "base.mode.channel.except",
        "base.mode.channel.invite-exception"
    ]
)]
async fn channel_mode_exceptions(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let channel = s.channel_name();
    let mapping = alice.state().casemapping();
    let mask = mapping.mask("reviewer!*@trusted.example");
    alice.join(&channel).await?;
    alice
        .mode(&channel)
        .ban_exception(&mask)
        .invite_exception(&mask)
        .await?;
    // List-mode numerics are the public evidence for stored exception masks.
    assert!(
        alice
            .list_modes(&channel, papo::ListMode::Except)
            .await?
            .iter()
            .any(|entry| entry.mask == mask)
    );
    assert!(
        alice
            .list_modes(&channel, papo::ListMode::Invex)
            .await?
            .iter()
            .any(|entry| entry.mask == mask)
    );

    alice
        .mode(&channel)
        .remove_ban_exception(&mask)
        .remove_invite_exception(&mask)
        .await?;
    assert!(
        alice
            .list_modes(&channel, papo::ListMode::Except)
            .await?
            .is_empty()
    );
    Ok(())
}

/// prevents an authenticated account from joining through an account extban
#[scenario(covers = ["ircv3.account-extban"])]
async fn account_extban(s: Scenario) -> papo_testing::Result<()> {
    if std::env::var("PAPO_TEST_PROFILE").unwrap_or_else(|_| "ergo-default".into()) != "solanum" {
        return s.skip("the selected profile does not expose Solanum's account extban syntax");
    }
    let alice = s.connect("alice").await?;
    let bob = s.connect_as_account("bob").await?;
    let account = crate::scenarios::support::logged_in_account(&bob.state()).clone();
    let channel = s.channel_name();
    alice.join(&channel).await?;
    alice
        .mode(&channel)
        .add_with('b', format!("$a:{account}"))
        .await?;

    let Error::Server(refusal) = bob.join(&channel).await.unwrap_err() else {
        panic!("account extban did not return a server refusal");
    };
    assert_eq!(refusal.refusal(), Refusal::BannedFromChannel);
    assert_eq!(refusal.code(), "474");
    assert!(bob.state().channel(&channel).is_none());
    Ok(())
}

/// blocks external messages and reports malformed channel mode changes
#[scenario(covers = [
    "base.mode.channel.no-external-messages",
    "base.mode.channel.err-unknown-mode",
    "base.mode.channel.err-invalid-param"
])]
async fn channel_mode_refusals(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let bob = s.connect("bob").await?;
    let channel = s.channel_name();
    alice.join(&channel).await?;
    alice.mode(&channel).add('n').await?;
    assert_eq!(
        alice.state().channel(&channel).unwrap().mode('n'),
        Some(ModeValue::Set)
    );

    let blocked = "this sender is outside the channel";
    assert_refusal(
        bob.privmsg(&channel, blocked).await.unwrap_err(),
        Refusal::CannotSendToChannel,
        "404",
    );
    alice
        .expect_none(|event| event.as_message().filter(|message| message.text == blocked))
        .await?;

    // These server numerics are the behavior under test and need raw MODE parameters.
    let unknown = alice
        .request_raw(papo::Line::new("MODE", [&channel, "+?"]))
        .await?;
    assert!(
        unknown.iter().any(|line| line.numeric() == Some(472)),
        "unexpected unknown-mode response: {unknown:?}"
    );
    let invalid = alice
        .request_raw(papo::Line::new("MODE", [&channel, "+l", "not-a-limit"]))
        .await?;
    assert!(
        invalid.iter().any(|line| line.numeric() == Some(696)),
        "unexpected invalid-mode response: {invalid:?}"
    );
    assert!(bob.state().channel(&channel).is_none());
    Ok(())
}

/// queries and changes invisible user mode while rejecting an unknown flag
#[scenario(covers = [
    "base.mode.user.query",
    "base.mode.user.invisible",
    "base.mode.user.err-unknown-flag"
])]
async fn user_modes(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let nick = alice.nick();

    // User-mode query and error numerics have no richer public result type.
    let initial = alice
        .request_raw(papo::Line::new("MODE", [nick.as_ref()]))
        .await?;
    assert!(initial.iter().any(|line| line.numeric() == Some(221)));
    if alice.state().user_modes().any(|mode| mode == 'i') {
        alice.mode(nick.clone()).remove('i').await?;
        let removed = alice
            .wait_for(|event| {
                event.as_mode_changed().filter(|changed| {
                    subject_name(&changed.subject) == nick.as_ref()
                        && changed
                            .changes
                            .iter()
                            .any(|change| change.mode == 'i' && change.action == ModeAction::Remove)
                })
            })
            .await?;
        assert!(removed.changes.iter().any(|change| {
            change.mode == 'i'
                && change.action == ModeAction::Remove
                && change.arg.clone().is_none()
        }));
    }

    alice.mode(nick.clone()).add('i').await?;
    let added = alice
        .wait_for(|event| {
            event.as_mode_changed().filter(|changed| {
                subject_name(&changed.subject) == nick.as_ref()
                    && changed
                        .changes
                        .iter()
                        .any(|change| change.mode == 'i' && change.action == ModeAction::Add)
            })
        })
        .await?;
    assert_eq!(subject_name(&added.subject), nick.as_ref());
    assert!(added.changes.iter().any(|change| {
        change.mode == 'i' && change.action == ModeAction::Add && change.arg.clone().is_none()
    }));
    assert!(alice.state().user_modes().any(|mode| mode == 'i'));

    let current = alice
        .request_raw(papo::Line::new("MODE", [nick.as_ref()]))
        .await?;
    assert!(current.iter().any(|line| {
        line.numeric() == Some(221) && line.params().iter().any(|param| param.contains('i'))
    }));
    let unknown = alice
        .request_raw(papo::Line::new("MODE", [nick.as_ref(), "+?"]))
        .await?;
    if !unknown.iter().any(|line| line.numeric() == Some(501)) {
        return Err(std::io::Error::other(format!(
            "unexpected unknown-user-mode response: {unknown:?}"
        ))
        .into());
    }
    Ok(())
}

/// enforces channel admission and messaging modes with typed refusals
#[scenario(covers = [
    "base.join.key-single",
    "base.join.err-bad-key",
    "base.join.err-banned",
    "base.join.err-invite-only",
    "base.join.err-channel-full",
    "base.mode.channel.ban",
    "base.mode.channel.key",
    "base.mode.channel.limit",
    "base.mode.channel.moderated",
    "base.mode.channel.secret"
])]
async fn channel_modes_and_join_refusals(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let mut carol = s.connect("carol").await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    let carol_nick = carol.nick();
    let key = "correct-horse-key";
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    alice.mode(&channel).add_with('k', key).add('s').await?;
    let modes = wait_channel_mode(&mut bob, &channel, 'k').await?;
    assert_eq!(subject_name(&modes.subject), channel);
    assert_actor_or_server(&modes.actor, alice_nick.as_ref());
    assert_eq!(modes.changes.len(), 2);
    let key_change = modes
        .changes
        .iter()
        .find(|change| change.mode == 'k')
        .unwrap();
    assert_eq!(key_change.action, ModeAction::Add);
    assert!(matches!(key_change.arg.clone(), Some(ModeArg::Value(value)) if value == key));
    let secret_change = modes
        .changes
        .iter()
        .find(|change| change.mode == 's')
        .unwrap();
    assert_eq!(secret_change.action, ModeAction::Add);
    assert_eq!(secret_change.arg, None);
    for peer in [&alice, &bob] {
        let state = peer.state();
        let channel_state = state.channel(&channel).unwrap();
        assert_eq!(channel_state.mode('k'), Some(ModeValue::Value(key)));
        assert_eq!(channel_state.mode('s'), Some(ModeValue::Set));
    }

    assert_refusal(
        carol.join(&channel).key("wrong key").await.unwrap_err(),
        Refusal::BadChannelKey,
        "475",
    );
    alice.mode(&channel).add_with('l', "2").await?;
    wait_channel_mode(&mut bob, &channel, 'l').await?;
    assert_refusal(
        carol.join(&channel).key(key).await.unwrap_err(),
        Refusal::ChannelIsFull,
        "471",
    );

    alice.mode(&channel).remove('l').await?;
    wait_channel_mode(&mut bob, &channel, 'l').await?;
    let mask = format!("{carol_nick}!*@*");
    alice
        .mode(&channel)
        .ban(alice.state().casemapping().mask(&mask))
        .await?;
    wait_channel_mode(&mut bob, &channel, 'b').await?;
    assert_refusal(
        carol.join(&channel).key(key).await.unwrap_err(),
        Refusal::BannedFromChannel,
        "474",
    );

    alice
        .mode(&channel)
        .unban(alice.state().casemapping().mask(mask))
        .add('i')
        .await?;
    wait_channel_mode(&mut bob, &channel, 'i').await?;
    assert_refusal(
        carol.join(&channel).key(key).await.unwrap_err(),
        Refusal::InviteOnlyChannel,
        "473",
    );
    alice.invite(&channel, carol_nick.clone()).await?;
    let invited = carol.wait_for(Event::as_invited).await?;
    assert_eq!(invited.nick, carol_nick);
    assert_eq!(invited.channel.as_ref(), channel);
    carol.join(&channel).key(key).await?;

    alice.mode(&channel).add('m').await?;
    wait_channel_mode(&mut bob, &channel, 'm').await?;
    assert_refusal(
        bob.privmsg(&channel, "blocked while unvoiced")
            .await
            .unwrap_err(),
        Refusal::CannotSendToChannel,
        "404",
    );
    alice.mode(&channel).voice(bob_nick).await?;
    wait_channel_mode(&mut bob, &channel, 'v').await?;
    bob.privmsg(&channel, "allowed after voice").await?;
    let delivered = carol.wait_for(Event::as_message).await?;
    assert_eq!(delivered.text, "allowed after voice");
    Ok(())
}

fn assert_actor_or_server(source: &Source, actor: &str) {
    match source {
        Source::User { nick, .. } => assert_eq!(nick, actor),
        Source::Server { name, .. } => assert!(!name.is_empty()),
        _ => panic!("mode source is neither a user nor a server"),
    }
}
