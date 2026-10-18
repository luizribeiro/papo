use std::{
    future::{Future, IntoFuture},
    pin::Pin,
    time::Duration,
};

use papo_proto::MessageRef;
use proptest::{prelude::*, strategy::ValueTree, test_runner::TestRunner};
use tokio::io::AsyncReadExt;

use super::test_support::*;

fn arbitrary_text() -> impl Strategy<Value = String> {
    proptest::collection::vec(any::<char>(), 0..768)
        .prop_map(|characters| characters.into_iter().collect())
}

fn inputs() -> Vec<(String, String, String)> {
    let mut values = vec![
        (String::new(), String::new(), String::new()),
        ("\r\n\0\x01".into(), ":tag".into(), "a b".into()),
        (":".into(), "+draft/example".into(), "\r\n\0".into()),
        (
            "é".repeat(4096),
            "+example.org/large".into(),
            "x".repeat(4095),
        ),
    ];
    let mut runner = TestRunner::deterministic();
    let strategy = (arbitrary_text(), arbitrary_text(), arbitrary_text());
    for _ in 0..16 {
        values.push(strategy.new_tree(&mut runner).unwrap().current());
    }
    values
}

async fn read_line(server: &mut DuplexStream) -> Vec<u8> {
    let mut line = Vec::new();
    loop {
        let byte = server.read_u8().await.unwrap();
        line.push(byte);
        if byte == b'\n' {
            return line;
        }
    }
}

fn validate_line(line: &[u8]) -> papo_proto::MessageRef<'_> {
    assert!(line.ends_with(b"\r\n"));
    let content = &line[..line.len() - 2];
    assert!(!content.contains(&b'\r'));
    assert!(!content.contains(&b'\n'));
    assert!(!content.contains(&0));
    let tag_len = content
        .strip_prefix(b"@")
        .and_then(|tagged| tagged.iter().position(|byte| *byte == b' '))
        .unwrap_or(0);
    let body_start = usize::from(tag_len != 0) * (tag_len + 2);
    assert!(line.len() - body_start <= 512);
    assert!(tag_len <= 4094);
    MessageRef::parse_bytes(line).unwrap()
}

type Action = Pin<Box<dyn Future<Output = Result<(), Error>>>>;

fn action<T: 'static, F>(future: F) -> Action
where
    F: IntoFuture<Output = Result<T, Error>> + 'static,
    F::IntoFuture: 'static,
{
    Box::pin(async move { future.await.map(drop) })
}

async fn check_action(make: impl FnOnce(super::Client) -> Action) {
    let (mut server, client, _events, driver) = scripted(config());
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
    let mut request = make(client);
    let mut lines = Vec::new();
    let result = tokio::select! {
        result = &mut request => Some(result),
        line = read_line(&mut server) => {
            lines.push(line);
            None
        }
    };
    while let Ok(line) =
        tokio::time::timeout(Duration::from_millis(1), read_line(&mut server)).await
    {
        lines.push(line);
    }
    for line in &lines {
        validate_line(line);
    }
    if let Some(result) = result {
        match result {
            Ok(()) => assert!(!lines.is_empty()),
            Err(error) => {
                assert!(matches!(
                    error,
                    Error::InvalidInput(_) | Error::Unsupported(_)
                ));
            }
        }
    }
    abort_task(driver).await;
}

#[derive(Clone, Copy)]
enum ChatKind {
    Privmsg,
    Notice,
    Action,
}

async fn check_chat(kind: ChatKind, text: String, key: String, value: String) {
    let (mut server, client, _events, driver) = scripted(config());
    let driver = tokio::spawn(driver);
    registration_burst(&mut server).await;
    let request = match kind {
        ChatKind::Privmsg => client.privmsg("#papo", &text),
        ChatKind::Notice => client.notice("#papo", &text),
        ChatKind::Action => client.action("#papo", &text),
    }
    .tag(key, value);
    let mut sending = tokio::spawn(async move { request.await });
    let mut lines = Vec::new();
    let result = loop {
        tokio::select! {
            result = &mut sending => break result.unwrap(),
            line = read_line(&mut server) => lines.push(line),
        }
    };
    while let Ok(line) =
        tokio::time::timeout(Duration::from_millis(1), read_line(&mut server)).await
    {
        lines.push(line);
    }

    if let Err(error) = result {
        assert!(matches!(
            error,
            Error::InvalidInput(_) | Error::Unsupported(_)
        ));
        assert!(lines.is_empty());
        abort_task(driver).await;
        return;
    }
    assert!(!lines.is_empty());
    let mut emitted = String::new();
    for line in &lines {
        let parsed = validate_line(line);
        emitted.push_str(parsed.param(1).unwrap());
    }
    let expected = text.replace('\n', "");
    if matches!(kind, ChatKind::Action) {
        assert_eq!(
            emitted
                .strip_prefix("\x01ACTION ")
                .unwrap()
                .strip_suffix('\x01')
                .unwrap(),
            expected
        );
    } else {
        assert_eq!(emitted, expected);
    }
    abort_task(driver).await;
}

#[tokio::test(flavor = "current_thread")]
async fn arbitrary_chat_input_is_rejected_or_round_trips_on_the_wire() {
    for (text, key, value) in inputs() {
        check_chat(ChatKind::Privmsg, text.clone(), key.clone(), value.clone()).await;
        check_chat(ChatKind::Notice, text.clone(), key.clone(), value.clone()).await;
        check_chat(ChatKind::Action, text, key, value).await;
    }
}

mod actions;
mod extension_actions;
mod remaining_actions;
