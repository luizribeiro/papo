//! Transcript coverage for incoming conversation classification.

mod support;

use papo_core::{Config, Conversation, Event, Subject};
use papo_proto::{Casemapping, InvalidIdentifier};
use support::Transcript;

fn assert_text_comparison(conversation: &Conversation, equal: &str, unequal: &str) {
    assert!(<Conversation as PartialEq<str>>::eq(conversation, equal));
    assert!(!<Conversation as PartialEq<str>>::eq(conversation, unequal));
    assert!(<Conversation as PartialEq<&str>>::eq(conversation, &equal));
    assert!(!<Conversation as PartialEq<&str>>::eq(
        conversation,
        &unequal
    ));
}

#[test]
fn compares_every_conversation_kind_with_text() {
    for mapping in [Casemapping::Rfc1459, Casemapping::Ascii] {
        assert_text_comparison(
            &Conversation::Channel(mapping.channel("#Papo")),
            "#papo",
            "#rust",
        );
    }
    assert_text_comparison(
        &Conversation::Private(Casemapping::Rfc1459.nick("bob")),
        "Bob",
        "alice",
    );
}

#[test]
fn exposes_and_compares_every_subject_kind() {
    let channel = Casemapping::Rfc1459.channel("#Papo[dev]");
    let nick = Casemapping::Rfc1459.nick("Alice[bot]");
    let subjects = [
        (Subject::Channel(channel.clone()), "#papo{dev}"),
        (Subject::User(nick.clone()), "alice{bot}"),
    ];

    for (subject, text) in subjects {
        assert!(subject == text);
        assert!(subject != "#elsewhere");
    }
    assert_eq!(Subject::Channel(channel.clone()).channel(), Some(&channel));
    assert_eq!(Subject::Channel(channel).nick(), None);
    assert_eq!(Subject::User(nick.clone()).nick(), Some(&nick));
    assert_eq!(Subject::User(nick).channel(), None);
}

#[test]
fn classifies_server_broadcast_and_private_conversations() {
    Transcript::new(Config::default().nick("papo"))
        .server(":ergo.test NOTICE * :*** Looking up your hostname...")
        .event(|event| {
            matches!(event, Event::Line(line) if line.conversation().is_none())
        })
        .server("NOTICE * :*** Checking Ident")
        .event(|event| {
            matches!(event, Event::Line(line) if line.conversation().is_none())
        })
        .server(":irc.solanum.example NOTICE papo :Server maintenance begins soon")
        .event(|event| {
            matches!(event, Event::Line(line)
                if line.conversation().is_none() && line.status().is_none())
        })
        .server(":irc.example NOTICE #staff :Services will restart shortly")
        .event(|event| {
            matches!(event, Event::Line(line)
                if matches!(line.conversation(), Some(Conversation::Channel(channel)) if channel == "#staff"))
        })
        .server(":irc.example NOTICE $*.example.org :Maintenance begins soon")
        .event(|event| {
            matches!(event, Event::Line(line)
                if line.conversation().is_none())
        })
        .server(":ergo.test WALLOPS :Network maintenance begins soon")
        .event(|event| {
            matches!(event, Event::Line(line)
                if line.conversation().is_none())
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
fn classifies_numerics_as_server_lines_without_a_source() {
    Transcript::new(Config::default().nick("papo"))
        .server("404 papo #papo :Cannot send to channel")
        .event(|event| {
            matches!(event, Event::Line(line)
                if line.conversation().is_none())
        })
        .no_event();
}

#[test]
fn separates_advertised_status_from_the_channel() {
    Transcript::new(Config::default().nick("papo"))
        .server(":ergo.test 005 papo CHANTYPES=# STATUSMSG=@+ :are supported")
        .server(":irc.example.net NOTICE @#ops :[Knock] by mallory!~m@203.0.113.9 (let me in)")
        .event(|event| {
            matches!(event, Event::Line(line)
                if line.status() == Some(['@'].as_slice())
                    && matches!(line.conversation(), Some(Conversation::Channel(channel)) if channel == "#ops"))
        })
        .server(":irc.example NOTICE @#chan :<mallory:#chan> text")
        .event(|event| {
            matches!(event, Event::Line(line)
                if line.status() == Some(['@'].as_slice())
                    && matches!(line.conversation(), Some(Conversation::Channel(channel)) if channel == "#chan"))
        })
        .server(":irc.example NOTICE @+#ops :Operators and voiced users only")
        .event(|event| {
            matches!(event, Event::Line(line)
                if line.status() == Some(['@', '+'].as_slice())
                    && matches!(line.conversation(), Some(Conversation::Channel(channel)) if channel == "#ops"))
        })
        .no_event();
}

#[test]
fn preserves_ops_only_addresses_across_incoming_line_kinds_and_history() {
    let is_ops = |status: Option<&[char]>, conversation: Option<&Conversation>| {
        status == Some(['@'].as_slice())
            && matches!(conversation, Some(Conversation::Channel(channel)) if channel == "#ops")
    };
    Transcript::new(Config::default().nick("papo"))
        .server(":ergo.test 005 papo CHANTYPES=# STATUSMSG=@+ :are supported")
        .server("@msgid=RfcBTzyxY120001 :alice!~a@cloak PRIVMSG @#ops :deploy complete")
        .event(|event| {
            matches!(event, Event::Line(line)
                if is_ops(line.status(), line.conversation()))
        })
        .server("@msgid=RfcBTzyxY120002 :alice!~a@cloak NOTICE @#ops :maintenance starts soon")
        .event(|event| {
            matches!(event, Event::Line(line)
                if is_ops(line.status(), line.conversation()))
        })
        .server(":alice!~a@cloak PRIVMSG @#ops :\u{1}VERSION\u{1}")
        .event(|event| {
            matches!(event, Event::Line(line)
                if is_ops(line.status(), line.conversation()))
        })
        .server("@+draft/react=👍;+reply=RfcBTzyxY120001 :bob!~b@cloak TAGMSG @#ops")
        .event(|event| {
            matches!(event, Event::TagMessage(line)
                if is_ops(line.status.as_deref(), Some(&line.conversation)))
        })
        .server("@+typing=active :bob!~b@cloak TAGMSG @#ops")
        .event(|event| {
            matches!(event, Event::TypingChanged(line)
                if is_ops(line.status.as_deref(), Some(&line.conversation)))
        })
        .server(":alice!~a@cloak REDACT @#ops RfcBTzyxY120001 :superseded")
        .event(|event| {
            matches!(event, Event::Redacted(line)
                if is_ops(line.status.as_deref(), Some(&line.conversation)))
        })
        .server(":ergo.test BATCH +history chathistory #ops")
        .server(
            "@batch=history;msgid=RfcBTzyxY120003 :alice!~a@cloak PRIVMSG @#ops :earlier deploy",
        )
        .server("@batch=history;+draft/react=👍;+reply=RfcBTzyxY120003 :bob!~b@cloak TAGMSG @#ops")
        .server("@batch=history :alice!~a@cloak REDACT @#ops RfcBTzyxY120003 :rolled back")
        .server(":ergo.test BATCH -history")
        .event(|event| {
            let Event::History { items, .. } = event else {
                return false;
            };
            items.iter().all(|item| {
                let (papo_core::HistoryItem::Message(line)
                | papo_core::HistoryItem::Reacted(line)
                | papo_core::HistoryItem::Redacted(line)) = item
                else {
                    return false;
                };
                line.as_ref().param(0) == Some("@#ops")
            })
        })
        .no_event();
}

#[test]
fn strips_status_only_when_the_remainder_is_a_channel() {
    Transcript::new(Config::default().nick("papo"))
        .server(":ergo.test 005 papo CHANTYPES=#+ STATUSMSG=@+ :are supported")
        .inspect(|session| {
            assert!(matches!(
                session.state().parse_target("+modeless"),
                Ok((None, Conversation::Channel(channel))) if channel == "+modeless"
            ));
            assert!(matches!(
                session.state().parse_target("@+modeless"),
                Ok((Some(status), Conversation::Channel(channel)))
                    if status == ['@'] && channel == "+modeless"
            ));
        })
        .server(":alice!~a@cloak PRIVMSG +modeless :meeting notes")
        .event(|event| {
            matches!(event, Event::Line(line)
                if line.status().is_none()
                    && matches!(line.conversation(), Some(Conversation::Channel(channel)) if channel == "+modeless"))
        })
        .server(":ergo.test 005 papo CHANTYPES=#& STATUSMSG=~&@%+ :are supported")
        .inspect(|session| {
            assert!(matches!(
                session.state().parse_target("&ops"),
                Ok((None, Conversation::Channel(channel))) if channel == "&ops"
            ));
            assert_eq!(
                session.state().parse_target("+alice"),
                Err(InvalidIdentifier::InvalidPrefix('+'))
            );
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
