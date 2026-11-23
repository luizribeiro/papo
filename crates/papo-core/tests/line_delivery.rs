//! Exercise unsolicited lines independently so one probe cannot affect the next.

use std::time::{Duration, Instant};

use papo_core::{Config, Session};
use papo_proto::LineRef;

fn check_delivery(line: &str, silent: &mut Vec<String>) {
    let now = Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_connected(now);
    for setup in [
        "CAP * LS :",
        ":ergo.test 001 papo :Welcome to Ergo",
        ":ergo.test 376 papo :End of /MOTD command",
        ":papo!~papo@localhost JOIN #papo",
        ":ergo.test 353 papo = #papo :@papo alice",
        ":ergo.test 366 papo #papo :End of /NAMES list.",
        ":ergo.test 315 papo #papo :End of /WHO list.",
    ] {
        session.handle_line(now, setup.as_bytes());
    }
    while session.poll_event().is_some() {}
    session.handle_line(now + Duration::from_secs(1), line.as_bytes());
    if session.poll_event().is_none() && !is_internal(&LineRef::parse(line).unwrap()) {
        silent.push(line.to_owned());
    }
}

fn is_internal(line: &LineRef<'_>) -> bool {
    match line.raw_command() {
        // ISUPPORT updates the negotiated server settings.
        "005"
        // The user mode snapshot populates our own mode set.
        | "221"
        // Clearing an already-clear away status is an idempotent acknowledgement.
        | "305"
        // Modes, creation time, topic, and topic setter/time seed the channel snapshot.
        | "324" | "329" | "332" | "333"
        // Only the first NAMES terminator can finish the join.
        | "366"
        // CAP errors drive negotiation and fallback.
        | "410"
        // MONITOR replies refresh watches and emit events for relevant transitions.
        | "730" | "731"
        // Subscription replies update the metadata subscription set.
        | "770" | "771" | "772"
        // SASL success is control traffic. An unsolicited success proves nothing.
        | "903"
        // Keepalive traffic is exchanged internally.
        | "PING" | "PONG" => true,
        // An opening batch buffers until closure or abandonment.
        "BATCH" => line.param(0).unwrap_or_default().starts_with('+'),
        // CAP NEW can advertise a capability without enabling it.
        "CAP" => line.param(1) == Some("NEW"),
        _ => false,
    }
}

#[test]
fn unsolicited_numerics_are_delivered() {
    let mut silent = Vec::new();
    for numeric in 0..=999 {
        for params in ["", " papo", " papo #papo :Scheduled maintenance"] {
            check_delivery(&format!(":ergo.test {numeric:03}{params}"), &mut silent);
        }
    }
    assert!(silent.is_empty(), "silent numerics:\n{}", silent.join("\n"));
}

#[test]
fn known_commands_are_delivered_from_each_source_shape() {
    let mut silent = Vec::new();
    for line in [
        "ACCOUNT alice-services",
        "ACK",
        "ADMIN ergo.test",
        "AUTHENTICATE +",
        "AWAY :Out to lunch",
        "BATCH +j8 netjoin relay.example ergo.test",
        "BOUNCER NETWORK 1 name=OfficeIRC",
        "CAP papo NEW :extended-monitor",
        "CHATHISTORY LATEST #papo * 10",
        "CHGHOST ~alice staff.example",
        "CONNECT relay.example 6697",
        "ERROR :Closing Link: maintenance",
        "EXTJWT #papo account",
        "FAIL JOIN CHANNEL_FULL #papo :Channel is full",
        "INFO ergo.test",
        "INVITE papo #staff",
        "ISON alice carol",
        "ISUPPORT WHOX",
        "JOIN #papo",
        "KICK #papo alice :Channel rules",
        "KILL alice :Requested by network staff",
        "LINKS ergo.test",
        "LIST #papo",
        "LUSERS",
        "MARKREAD #papo timestamp=2026-11-22T10:15:00.000Z",
        "METADATA #papo avatar * :https://example.com/avatar.png",
        "MODE #papo +i",
        "MONITOR + carol",
        "MOTD ergo.test",
        "NAMES #papo",
        "NICK alicia",
        "NOTE WHOIS ACCOUNT alice :Account information",
        "NOTICE #papo :Scheduled maintenance",
        "OPER network-admin",
        "PART #papo :See you tomorrow",
        "PASS",
        "PERSISTENCE ON",
        "PING :ergo.test",
        "PONG ergo.test :papo",
        "PRIVMSG #papo :Meeting starts in ten minutes",
        "QUIT :See you tomorrow",
        "REDACT #papo office-1 :Wrong channel",
        "REGISTER alice *",
        "REHASH",
        "RELAYMSG #papo alice/matrix :Hello from Matrix",
        "RENAME #papo #staff :Team channel",
        "RESTART",
        "SETNAME :Alice Smith",
        "SQUIT relay.example :Scheduled maintenance",
        "STATS u",
        "TAGMSG #papo",
        "TIME ergo.test",
        "TOPIC #papo :Weekly planning",
        "TRACE ergo.test",
        "USER alice 0 * :Alice Smith",
        "USERHOST alice",
        "VERIFY alice",
        "VERSION ergo.test",
        "WALLOPS :Scheduled maintenance",
        "WARN JOIN DEPRECATED #papo :Channel is moving",
        "WEBPUSH",
        "WHO #papo",
        "WHOIS alice",
        "WHOWAS alice",
    ] {
        for source in [
            ":alice!~alice@staff.example ",
            ":carol!~carol@users.example ",
            ":ergo.test ",
            "",
        ] {
            check_delivery(&format!("{source}{line}"), &mut silent);
        }
    }
    assert!(silent.is_empty(), "silent commands:\n{}", silent.join("\n"));
}
