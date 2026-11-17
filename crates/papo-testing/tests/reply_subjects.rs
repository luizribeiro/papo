//! A person a WHO or WHOIS reply describes is the one the server described: a send through the
//! reply's handle never reaches a later holder of the nickname.
//!
//! A labeled-response batch is dispatched when it closes, so a departure inside it has been
//! applied before papo maps the reply.

mod support {
    pub mod direct;
}

use std::time::Duration;

use papo::{Error, Event, Reconnect};
use papo_testing::{Wire, wire};
use support::direct::{connected, next_active_event};

const SECRET: &str = "the deploy token is hunter2";

struct Reply {
    request: &'static str,
    rows: &'static [&'static str],
}

const REPLIES: [Reply; 3] = [
    Reply {
        request: "WHO alice %tcuhnfar,152",
        rows: &[
            ":offline.test 354 papo 152 #review ~alice cloak.example alice H alice-account :Alice Liddell",
            ":offline.test 315 papo alice :End of WHO list",
        ],
    },
    Reply {
        request: "WHO alice",
        rows: &[
            ":offline.test 352 papo #review ~alice cloak.example offline.test alice H :0 Alice Liddell",
            ":offline.test 315 papo alice :End of WHO list",
        ],
    },
    Reply {
        request: "WHOIS alice",
        rows: &[
            ":offline.test 311 papo alice ~alice cloak.example * :Alice Liddell",
            ":offline.test 330 papo alice alice-account :is logged in as",
            ":offline.test 318 papo alice :End of /WHOIS list",
        ],
    },
];

async fn connect(reply: &Reply, shared_channel: bool) -> (papo::Client, papo::Events, Wire) {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["account-tag", "batch", "labeled-response", "message-tags"])
        .await;
    connected(&mut events).await;
    if reply.request.contains('%') {
        wire.server(":offline.test 005 papo WHOX :are supported by this server")
            .await;
        while !client.snapshot().isupport().whox() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
    if shared_channel {
        for line in [
            ":papo!~papo@localhost JOIN #review",
            "@account=alice-account :alice!~alice@cloak.example JOIN #review",
        ] {
            wire.server(line).await;
            while !matches!(next_active_event(&mut events).await, Event::Joined(_)) {}
        }
    }
    (client, events, wire)
}

/// Reads client lines up to `expected` and returns its label.
async fn label_of(wire: &mut Wire, expected: &str) -> String {
    loop {
        let line = wire.next_sent().await;
        if let Some((tags, command)) = line.split_once(' ')
            && command == expected
            && let Some(label) = tags.strip_prefix("@label=")
        {
            return label.to_owned();
        }
        assert!(!line.contains("PRIVMSG"), "sent {line}");
    }
}

/// Answers `reply` in one labeled batch with `between` before its end, then sends through it,
/// answering papo's WHO check with `holder`'s account when given.
async fn send_after(
    reply: &Reply,
    client: &papo::Client,
    wire: &mut Wire,
    between: &[&str],
    holder: Option<&str>,
) -> (Option<String>, Result<(), Error>) {
    let whois = reply.request.starts_with("WHOIS");
    let sending = tokio::spawn({
        let client = client.clone();
        async move {
            let alice = if whois {
                client.whois("alice").await?.unwrap().user
            } else {
                client.who("alice").await?.remove(0).user
            };
            client.privmsg(alice, SECRET).await.map(drop)
        }
    });
    let label = label_of(wire, reply.request).await;
    let (end, rows) = reply.rows.split_last().unwrap();
    wire.server(format!(
        "@label={label} :offline.test BATCH +reply labeled-response"
    ))
    .await;
    for line in rows.iter().chain(between).chain([end]) {
        wire.server(format!("@batch=reply {line}")).await;
    }
    wire.server(":offline.test BATCH -reply").await;
    if let Some(account) = holder {
        let label = label_of(wire, "WHO alice %na").await;
        for line in [
            format!("@label={label} :offline.test 354 papo alice {account}"),
            format!("@label={label} :offline.test 315 papo alice :End of WHO list"),
        ] {
            wire.server(line).await;
        }
    }
    let sent = tokio::time::timeout(Duration::from_millis(50), wire.next_sent())
        .await
        .ok();
    (sent, sending.await.unwrap())
}

fn check(case: &str, (sent, result): (Option<String>, Result<(), Error>), to: Option<&str>) {
    let expected = to.map_or((None, "Err(TargetGone)"), |nick| {
        (Some(format!("PRIVMSG {nick} :{SECRET}")), "Ok(())")
    });
    assert_eq!((sent, format!("{result:?}").as_str()), expected, "{case}");
}

#[tokio::test]
async fn a_reply_subject_seen_leaving_or_renaming_is_never_the_next_holder() {
    let quit = [
        ":alice!~alice@cloak.example QUIT :Quit: bye",
        ":alice!~mallory@users.example JOIN #review",
    ];
    let rename = [
        ":alice!~alice@cloak.example NICK alice_away",
        ":mallory!~mallory@users.example JOIN #review",
        ":mallory!~mallory@users.example NICK alice",
    ];
    for reply in &REPLIES {
        for (between, to) in [(&quit[..], None), (&rename[..], Some("alice_away"))] {
            let (client, _events, mut wire) = connect(reply, true).await;
            let outcome = send_after(reply, &client, &mut wire, between, None).await;
            check(
                &format!("{} then {}", reply.request, between[0]),
                outcome,
                to,
            );
        }
    }
}

#[tokio::test]
async fn an_unseen_handoff_reaches_only_a_holder_of_the_replys_account() {
    for reply in [&REPLIES[0], &REPLIES[2]] {
        for (holder, to) in [
            ("0", None),
            ("mallory-account", None),
            ("alice-account", Some("alice")),
        ] {
            let (client, _events, mut wire) = connect(reply, false).await;
            let outcome = send_after(reply, &client, &mut wire, &[], Some(holder)).await;
            check(&format!("{} then {holder}", reply.request), outcome, to);
        }
    }
}
