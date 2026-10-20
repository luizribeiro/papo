//! Transcript coverage for incoming conversation classification.

mod support;

use papo_core::{Config, Conversation, Event};
use support::Transcript;

#[test]
fn classifies_server_broadcast_and_private_conversations() {
    Transcript::new(Config::default().nick("papo"))
        .server(":ergo.test NOTICE * :*** Looking up your hostname...")
        .event(|event| {
            matches!(event, Event::Line(line) if line.conversation() == Some(&Conversation::Server))
        })
        .server("NOTICE * :*** Checking Ident")
        .event(|event| {
            matches!(event, Event::Line(line) if line.conversation() == Some(&Conversation::Server))
        })
        .server(":irc.solanum.example NOTICE papo :Server maintenance begins soon")
        .event(|event| {
            matches!(event, Event::Line(line)
                if line.conversation() == Some(&Conversation::Server) && line.status().is_none())
        })
        .server(":irc.example NOTICE #staff :Services will restart shortly")
        .event(|event| {
            matches!(event, Event::Line(line)
                if matches!(line.conversation(), Some(Conversation::Channel(channel)) if channel == "#staff"))
        })
        .server(":irc.example NOTICE $*.example.org :Maintenance begins soon")
        .event(|event| {
            matches!(event, Event::Line(line)
                if matches!(line.conversation(), Some(Conversation::Broadcast(mask)) if mask == "$*.example.org"))
        })
        .server(":ergo.test WALLOPS :Network maintenance begins soon")
        .event(|event| {
            matches!(event, Event::Line(line)
                if line.conversation() == Some(&Conversation::Server))
        })
        .server(":bob!~bob@users.example PRIVMSG papo :Are you around?")
        .event(|event| {
            matches!(event, Event::Line(line)
                if matches!(line.conversation(), Some(Conversation::Private(nick)) if nick == "bob"))
        })
        .server(":papo!~papo@localhost PRIVMSG papo :remember the deploy window")
        .event(|event| {
            matches!(event, Event::Line(line)
                if matches!(line.conversation(), Some(Conversation::Private(nick)) if nick == "papo"))
        })
        .no_event();
}

#[test]
fn separates_advertised_status_from_the_channel() {
    Transcript::new(Config::default().nick("papo"))
        .server(":ergo.test 005 papo CHANTYPES=# STATUSMSG=@+ :are supported")
        .server(":alice!~a@cloak NOTICE @#ops :Incident channel is restricted")
        .event(|event| {
            matches!(event, Event::Line(line)
                if line.status() == Some(['@'].as_slice())
                    && matches!(line.conversation(), Some(Conversation::Channel(channel)) if channel == "#ops"))
        })
        .no_event();
}

#[test]
fn does_not_classify_an_unadvertised_status_prefix() {
    Transcript::new(Config::default().nick("papo"))
        .server(":solanum.example 005 papo CHANTYPES=# STATUSMSG=+ :are supported")
        .server(":alice!~a@cloak NOTICE @#ops :This target is invalid here")
        .event(|event| {
            matches!(event, Event::Line(line)
                if line.conversation().is_none() && line.status().is_none())
        })
        .no_event();
}
