//! Transcript coverage for incoming channel invitations.

mod support;

use papo_core::{Config, Event};
use support::Transcript;

#[test]
fn reports_direct_and_notified_invitations() {
    Transcript::new(Config::default())
        .server(":emily!~emily@localhost INVITE papo :#papo")
        .event(|event| {
            matches!(
                event,
                Event::Invited { channel, nick, by }
                    if channel == "#papo" && nick == "papo"
                        && by == "emily!~emily@localhost"
            )
        })
        .server(":alice!~alice@staff.example INVITE bob #staff")
        .event(|event| {
            matches!(
                event,
                Event::Invited { channel, nick, by }
                    if channel == "#staff" && nick == "bob"
                        && by == "alice!~alice@staff.example"
            )
        })
        .no_client();
}

#[test]
fn leaves_malformed_invitations_untyped() {
    Transcript::new(Config::default())
        .server(":emily!~emily@localhost INVITE papo")
        .event(|event| matches!(event, Event::Message(message) | Event::Line(message) if message.as_ref().command_is("INVITE")));
}
