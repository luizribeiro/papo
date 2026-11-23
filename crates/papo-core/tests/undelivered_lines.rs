//! Transcript coverage for unsolicited presence and metadata.

mod support;

use papo_core::{AwayStatus, Config, Event, Party};
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
