use crate::driver::test_support::*;
use std::fmt::Write as _;

const TARGET: &str = "kzhqp4zbpva7jvgd9gc223pdma";

#[tokio::test(flavor = "current_thread")]
async fn unsolicited_history_reaches_the_application_even_at_disconnect() {
    let caps = "batch draft/chathistory server-time";
    let (mut server, _client, mut events, driver) = scripted(config().wanted_caps(caps.split(' ')));
    let task = tokio::spawn(driver);
    within(complete_connect_caps(&mut server, &mut events, caps, caps)).await;

    let assert_replay = |event, text, exhausted| {
        let Some(Event::History(page)) = event else {
            panic!("expected a history page containing {text:?}, got {event:?}");
        };
        assert_eq!(page.reason, crate::HistoryReason::Replay);
        assert_eq!(page.exhausted, exhausted);
        assert!(
            matches!(page.items.as_slice(), [Event::Message(message)]
                if message.text == text && message.is_from_history()),
            "unexpected history items: {:?}",
            page.items
        );
    };

    server_line(
        &mut server,
        b"@draft/chathistory-end;time=2026-11-23T10:48:40.521Z :ergo.test BATCH +2 chathistory #papo\r\n\
          @batch=2;time=2026-11-23T10:38:24.901Z;msgid=c7ub4kcsfnqbgq5ijpzpkx5v4a :emily!~e@localhost PRIVMSG #papo :release window opened\r\n\
          @time=2026-11-23T10:48:40.522Z :ergo.test BATCH -2\r\n",
    )
    .await;
    assert_replay(
        within(next_event(&mut events)).await,
        "release window opened",
        true,
    );

    server_line(
        &mut server,
        b"@time=2026-11-23T10:48:41.000Z :ergo.test BATCH +3 chathistory #papo\r\n\
          @batch=3;time=2026-11-23T10:45:07.880Z;msgid=6hr3dgdrjsy5dnpmsxwq3a8b2e :emily!~e@localhost PRIVMSG #papo :database migration completed\r\n",
    )
    .await;
    drop(server);

    assert_replay(
        within(next_event(&mut events)).await,
        "database migration completed",
        false,
    );
    assert!(matches!(
        within(next_event(&mut events)).await,
        Some(Event::Disconnected(_))
    ));
    assert!(matches!(
        within(task).await.unwrap(),
        Err(Error::Disconnected(_))
    ));
}

fn ergo_page(label: usize, items: &[&str]) -> String {
    let mut page = format!(
        "@label={label} :ergo.test BATCH +r{label} labeled-response\r\n@batch=r{label} :ergo.test BATCH +h{label} chathistory #papo\r\n"
    );
    for item in items {
        write!(page, "@batch=h{label};{item}\r\n").unwrap();
    }
    write!(
        page,
        "@batch=r{label} :ergo.test BATCH -h{label}\r\n:ergo.test BATCH -r{label}\r\n"
    )
    .unwrap();
    page
}

#[tokio::test(flavor = "current_thread")]
async fn a_message_fetch_keeps_only_the_item_with_exactly_that_id() {
    let cases: [(&str, &[&str], Option<&str>); 4] = [
        (
            "found between its neighbours",
            &[
                "time=2026-11-15T09:12:01.104Z;msgid=c7ub4kcsfnqbgq5ijpzpkx5v4a :emily!~e@localhost PRIVMSG #papo :release window opened",
                "time=2026-11-15T09:12:03.512Z;msgid=kzhqp4zbpva7jvgd9gc223pdma :emily!~e@localhost PRIVMSG #papo :database migration started",
                "time=2026-11-15T09:12:07.880Z;msgid=6hr3dgdrjsy5dnpmsxwq3a8b2e :emily!~e@localhost PRIVMSG #papo :database migration completed",
            ],
            Some("database migration started"),
        ),
        (
            "nearer items reference or resemble the id",
            &[
                "time=2026-11-15T09:12:04.001Z;msgid=t2xt7zrn8b6ny5i6uk2hqq8cxs;+draft/react=👍;+reply=kzhqp4zbpva7jvgd9gc223pdma :alice!~a@localhost TAGMSG #papo",
                "time=2026-11-15T09:12:04.002Z;msgid=KZHQP4ZBPVA7JVGD9GC223PDMA :alice!~a@localhost PRIVMSG #papo :same id in another case",
                "time=2026-11-15T09:12:03.512Z;msgid=kzhqp4zbpva7jvgd9gc223pdma :emily!~e@localhost PRIVMSG #papo :database migration started",
            ],
            Some("database migration started"),
        ),
        (
            "page holds only neighbours",
            &[
                "time=2026-11-15T09:12:01.104Z;msgid=c7ub4kcsfnqbgq5ijpzpkx5v4a :emily!~e@localhost PRIVMSG #papo :release window opened",
                "time=2026-11-15T09:12:07.880Z;msgid=6hr3dgdrjsy5dnpmsxwq3a8b2e :emily!~e@localhost PRIVMSG #papo :database migration completed",
            ],
            None,
        ),
        ("empty page", &[], None),
    ];
    let config = config().wanted_caps([
        "draft/chathistory",
        "labeled-response",
        "batch",
        "server-time",
    ]);
    let (mut server, client, mut events, driver) = scripted(config);
    let task = tokio::spawn(driver);
    complete_connect_history(&mut server, &mut events).await;
    for (label, (name, items, expected)) in cases.into_iter().enumerate() {
        let fetch = tokio::spawn({
            let client = client.clone();
            async move { client.history("#papo").message(TARGET).await }
        });
        client_line(
            &mut server,
            format!("@label={label} CHATHISTORY AROUND #papo msgid={TARGET} 3\r\n").as_bytes(),
        )
        .await;
        server_line(&mut server, ergo_page(label, items).as_bytes()).await;
        let found = within(fetch).await.unwrap().unwrap();
        let text = match &found {
            Some(Event::Message(message)) => Some(message.text.as_str()),
            None => None,
            Some(other) => panic!("{name}: unexpected item {other:?}"),
        };
        assert_eq!(text, expected, "{name}");
    }
    abort_task(task).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_message_fetch_needs_negotiated_chathistory() {
    let (mut server, client, mut events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    assert!(matches!(
        within(client.history("#papo").message(TARGET)).await,
        Err(Error::Unsupported(crate::Unsupported {
            feature: crate::Feature::History,
            reason: crate::UnsupportedReason::NotOffered,
            ..
        }))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(1), server.read_u8())
            .await
            .is_err()
    );
    abort_task(task).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_refused_message_fetch_is_a_server_error() {
    let config = config().wanted_caps([
        "draft/chathistory",
        "labeled-response",
        "batch",
        "server-time",
    ]);
    let (mut server, client, mut events, driver) = scripted(config);
    let task = tokio::spawn(driver);
    complete_connect_history(&mut server, &mut events).await;
    let fetch = tokio::spawn({
        let client = client.clone();
        async move { client.history("#papo").message(TARGET).await }
    });
    client_line(
        &mut server,
        format!("@label=0 CHATHISTORY AROUND #papo msgid={TARGET} 3\r\n").as_bytes(),
    )
    .await;
    server_line(
        &mut server,
        b"@label=0 :ergo.test FAIL CHATHISTORY INVALID_TARGET AROUND #papo :Messages could not be retrieved\r\n",
    )
    .await;
    assert!(matches!(
        within(fetch).await.unwrap(),
        Err(Error::Server(error)) if error.code() == "INVALID_TARGET"
    ));
    abort_task(task).await;
}
