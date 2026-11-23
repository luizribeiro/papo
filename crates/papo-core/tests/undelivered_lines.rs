//! Transcript coverage for unsolicited presence and metadata.

mod support;

use papo_core::{AccountStatus, AwayStatus, Config, Event, Party};
use papo_proto::Line;
use support::Transcript;

#[test]
fn away_reply_for_an_unseen_user_is_delivered() {
    Transcript::new(Config::default())
        .connect_default()
        .server(":ergo.test CAP * LS :")
        .client("CAP END")
        .welcome(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 376 papo :End of /MOTD command")
        .event(|event| matches!(event, Event::Connected(_)))
        .send(Line::new("PRIVMSG").with_params(["carol", "are you around?"]))
        .client("PRIVMSG carol :are you around?")
        .server(":ergo.test 301 papo carol :Gone fishing until Monday")
        .event(|event| {
            matches!(event, Event::AwayChanged { actor, nick, away, actor_party: Party::Other, .. }
                if actor == "carol" && nick == "carol"
                    && matches!(away, AwayStatus::Away { message: Some(text), .. }
                        if text == "Gone fishing until Monday"))
        })
        .no_event()
        .inspect(|session| assert!(session.state().user("carol").is_none()));
}

#[test]
fn whoisbot_outside_whois_is_delivered() {
    Transcript::new(Config::default())
        .server(":ergo.test 335 papo carol :is a Bot")
        .event(|event| {
            matches!(event, Event::Line(line) if line.as_ref().command_is("335")
                && line.as_ref().param(1) == Some("carol"))
        })
        .no_event()
        .inspect(|session| assert!(session.state().user("carol").is_none()));
}

#[test]
fn presence_lines_for_untracked_users_are_delivered() {
    let prefix = "alice!~alice@users.example";
    Transcript::new(Config::default())
        .connect_default()
        .server(":ergo.test CAP * LS :account-notify away-notify extended-join no-implicit-names")
        .client("CAP REQ :extended-join away-notify account-notify no-implicit-names")
        .server(":ergo.test CAP * ACK :account-notify away-notify extended-join no-implicit-names")
        .client("CAP END")
        .welcome(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 376 papo :End of /MOTD command")
        .event_until(|event| matches!(event, Event::Connected { .. }))
        .server(":papo!~papo@localhost JOIN #papo * :papo")
        .client("NAMES #papo")
        .event_until(|event| matches!(event, Event::Joined { .. }))
        .server(":alice!~alice@users.example AWAY :Gone fishing until Monday")
        .event(|event| {
            matches!(event, Event::AwayChanged { actor, nick, away, actor_party: Party::Other, .. }
                if actor == prefix && nick == "alice"
                    && matches!(away, AwayStatus::Away { message: Some(text), .. }
                        if text == "Gone fishing until Monday"))
        })
        .server(":alice!~alice@users.example ACCOUNT alice-services")
        .event(|event| {
            matches!(event, Event::AccountChanged { actor, nick, account, actor_party: Party::Other, .. }
                if actor == prefix && nick == "alice"
                    && matches!(account, AccountStatus::LoggedIn(name) if name == "alice-services"))
        })
        .no_event()
        .inspect(|session| assert!(session.state().user("alice").is_none()));
}

#[test]
fn postponed_metadata_sync_is_delivered() {
    Transcript::new(Config::default())
        .advance(std::time::Duration::from_secs(1))
        .server("@time=2026-11-22T10:15:00.000Z :ergo.test 774 papo #papo 30")
        .event(|event| {
            // A deferral is a retry hint, even when no sync request is in flight.
            matches!(event, Event::MetadataSyncLater { target, retry_after: Some(delay), line, .. }
                if target == "#papo" && delay.as_secs() == 30
                    && line.as_ref().raw_source() == Some("ergo.test")
                    && line.as_ref().tag("time").as_deref() == Some("2026-11-22T10:15:00.000Z"))
        })
        .no_event();
}

#[test]
fn prefixless_topic_and_modes_update_state_and_surface_the_line() {
    Transcript::new(Config::default())
        .server(":ergo.test 005 papo CHANMODES=Ibe,k,fl,CEMRUimnstu :are supported")
        .server(":papo!~papo@localhost JOIN #papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server("TOPIC #papo :Weekly planning")
        .event(|event| {
            matches!(event, Event::Line(line) if line.as_ref().command_is("TOPIC")
                && line.as_ref().source().is_none()
                && line.as_ref().param(1) == Some("Weekly planning"))
        })
        .server("MODE #papo +ik team-room")
        .event(|event| {
            matches!(event, Event::Line(line) if line.as_ref().command_is("MODE")
                && line.as_ref().source().is_none()
                && format!("{line:?}").contains("<redacted>")
                && !format!("{line:?}").contains("team-room"))
        })
        .no_event()
        .inspect(|session| {
            let channel = session.state().channel("#papo").unwrap();
            assert_eq!(channel.topic(), Some("Weekly planning"));
            assert_eq!(channel.mode('i'), Some(papo_core::ModeValue::Set));
            assert!(channel.key().is_some());
        });
}
