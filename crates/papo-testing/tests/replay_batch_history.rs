//! History provenance on the payloads delivered from replay batches.

use std::time::Duration;

use papo::{Client, Event, RateLimit, Reconnect};
use papo_testing::wire;

#[tokio::test(start_paused = true)]
async fn batch_provenance_agrees_for_single_rows_and_nested_multiline() {
    for (server, kind, target, historical) in [
        ("ergo.test", "chathistory", " #papo", true),
        ("soju.example", "draft/chathistory", " #papo", true),
        ("ergo.test", "draft/chathistory-targets", "", true),
        ("ergo.test", "draft/event-playback", " #papo", true),
        ("znc.example", "znc.in/playback", " #papo", true),
        ("soju.example", "soju.im/search", "", true),
        ("bouncer.example", "example.org/sync", " #papo", false),
    ] {
        let (_client, mut events, mut wire) = wire(
            Client::builder("irc://offline.test", "papo")
                .rate_limit(RateLimit::unlimited())
                .reconnect(Reconnect::never()),
        );
        wire.register(&[
            "batch",
            "draft/chathistory",
            "draft/event-playback",
            "draft/multiline=max-bytes=4096,max-lines=100",
            "message-tags",
            "server-time",
        ])
        .await;
        let (row, expected) = if kind == "draft/chathistory-targets" {
            (
                format!(":{server} CHATHISTORY TARGETS #papo 2026-10-15T12:00:00.000Z"),
                "#papo",
            )
        } else {
            (
                ":alice!~a@client.example PRIVMSG #papo :release is ready".to_owned(),
                "release is ready",
            )
        };
        for line in [
            ":papo!~u@2j8y98eazaf5w.irc JOIN #papo".to_owned(),
            format!(":{server} BATCH +replay {kind}{target}"),
            format!("@batch=replay;time=2026-10-15T12:00:00.000Z;msgid=6tiv6pw8a7d5mhn77ai2vm5jq2 {row}"),
            "@batch=replay;time=2026-10-15T12:00:05.000Z;msgid=fkggsh3fbnpgx8hzy9ah8u :alice!~a@client.example BATCH +paste draft/multiline #papo".to_owned(),
            "@batch=paste :alice!~a@client.example PRIVMSG #papo :release notes".to_owned(),
            "@batch=paste :alice!~a@client.example PRIVMSG #papo :second line".to_owned(),
            "@batch=replay :alice!~a@client.example BATCH -paste".to_owned(),
            format!(":{server} BATCH -replay"),
            ":offline.test NOTICE papo :end of scripted traffic".to_owned(),
        ] {
            wire.server(line).await;
        }

        let mut payloads = Vec::new();
        loop {
            let (event, _) = tokio::time::timeout(Duration::from_secs(60), events.next())
                .await
                .expect("the scripted traffic ends with a notice")
                .expect("the driver keeps running");
            match event {
                Event::ServerNotice(_) => break,
                Event::History(page) => payloads.extend(page.items),
                Event::Other(ref line) if matches!(line.command(), "PRIVMSG" | "CHATHISTORY") => {
                    payloads.push(event);
                }
                Event::Message(_) => payloads.push(event),
                _ => {}
            }
        }
        let rows: Vec<_> = payloads
            .iter()
            .map(|event| match event {
                Event::Other(line) => (line.get(1).unwrap(), line.is_from_history()),
                Event::Message(line) => (line.text.as_str(), line.is_from_history()),
                _ => panic!("unexpected payload in {kind}: {event:?}"),
            })
            .collect();
        assert_eq!(
            rows,
            [
                (expected, historical),
                ("release notes\nsecond line", historical)
            ],
            "{kind}"
        );
    }
}
