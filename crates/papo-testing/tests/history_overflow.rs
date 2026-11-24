//! A history page that outgrows the batch line limit must stay history.

use std::time::Duration;

use papo::{Client, Event, RateLimit, Reconnect};
use papo_testing::wire;

#[tokio::test]
async fn the_rest_of_an_oversized_history_page_is_not_live_chat() {
    let (client, mut events, mut wire) = wire(
        Client::builder("irc://offline.test", "papo")
            .rate_limit(RateLimit::unlimited())
            .event_capacity(20_000)
            .reconnect(Reconnect::never()),
    );
    wire.register(&[
        "account-notify",
        "account-tag",
        "batch",
        "draft/chathistory",
        "draft/multiline=max-bytes=4096,max-lines=100",
        "extended-join",
        "labeled-response",
        "message-tags",
        "server-time",
    ])
    .await;
    for line in [
        ":papo!~u@2j8y98eazaf5w.irc JOIN #review * :Papo",
        "@account=alice :alice!~u@8fq2mzkq5hcsa.irc JOIN #review alice :Alice",
    ] {
        wire.server(line).await;
    }

    let request = tokio::spawn(async move { client.history("#review").latest().limit(200).await });
    assert_eq!(
        wire.next_sent().await,
        "@label=0 CHATHISTORY LATEST #review * 200"
    );
    // The shape Ergo 2.19.1 sends: the label on the chathistory batch, each paste a nested
    // multiline batch whose opener carries the tags. 120 pastes of 100 lines is 12000 lines.
    wire.server(
        "@draft/chathistory-end;time=2026-11-23T10:48:40.521Z;label=0 :offline.test BATCH +2 chathistory #review",
    )
    .await;
    for paste in 0..120 {
        wire.server(format!(
            "@batch=2;time=2026-11-22T09:{:02}:{:02}.000Z;msgid=fkggsh3fbnpgx8hzy9ah8u{paste:04};account=alice :alice!~u@8fq2mzkq5hcsa.irc BATCH +p{paste} draft/multiline #review",
            paste / 60,
            paste % 60,
        ))
        .await;
        for row in 0..100 {
            wire.server(format!(
                "@batch=p{paste} :alice!~u@8fq2mzkq5hcsa.irc PRIVMSG #review :paste {paste} row {row}"
            ))
            .await;
        }
        wire.server(format!(
            "@batch=2 :alice!~u@8fq2mzkq5hcsa.irc BATCH -p{paste}"
        ))
        .await;
    }
    wire.server("@time=2026-11-23T10:48:41.000Z :offline.test BATCH -2")
        .await;
    wire.server(":offline.test NOTICE papo :end of scripted traffic")
        .await;

    let mut late_rows = 0;
    loop {
        let (event, _) = tokio::time::timeout(Duration::from_secs(120), events.next())
            .await
            .expect("the scripted traffic ends with a notice")
            .expect("the driver keeps running");
        match event {
            Event::ServerNotice(_) => break,
            Event::AccountChanged(changed) => {
                panic!("replay changed a present account: {changed:?}")
            }
            Event::Message(message) => assert!(message.is_from_history(), "{message:?}"),
            Event::Notice(notice) => assert!(notice.is_from_history(), "{notice:?}"),
            Event::Incomplete(incomplete) => {
                assert!(incomplete.is_from_history(), "{incomplete:?}");
            }
            Event::Other(other) if other.command() == "PRIVMSG" => {
                assert!(other.is_from_history(), "{other:?}");
                late_rows += 1;
            }
            _ => {}
        }
    }
    assert_eq!(late_rows, 1_999);
    request.abort();
}
