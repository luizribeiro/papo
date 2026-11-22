//! Highest-layer regression for public sends that point at an ops-only line.

use papo::{InvalidInput, Line, Reconnect};
use papo_testing::wire;

const ID: &str = "z6apvbu245cpg8t9b48v5bdmqs";

async fn next_restricted_message(
    events: &mut papo::Events,
) -> (papo::Prefix, papo::event::Message) {
    loop {
        match events.next().await.map(|(event, _)| event) {
            Some(papo::Event::Restricted(papo::Restricted {
                audience,
                line: papo::RestrictedLine::Message(message),
                ..
            })) => return (audience, message),
            Some(papo::Event::Disconnected(disconnected)) => panic!("disconnected: {disconnected}"),
            Some(_) => {}
            None => panic!("driver stopped while waiting for a message"),
        }
    }
}

fn widens_ops<T: std::fmt::Debug>(result: &Result<T, papo::Error>) -> bool {
    matches!(
        result,
        Err(papo::Error::InvalidInput(InvalidInput::WiderThanReferenced { referenced, .. }))
            if referenced.symbol() == '@'
    )
}

#[tokio::test]
async fn public_answers_to_an_ops_only_line_never_reach_the_wire() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://hostile.offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[
        "batch",
        "draft/message-redaction",
        "labeled-response",
        "message-tags",
    ])
    .await;
    wire.server(":offline.test 005 papo STATUSMSG=~&@%+ :are supported by this server")
        .await;
    wire.server(format!(
        "@time=2026-11-15T00:13:53.319Z;msgid={ID} :oppy!~u@87gpv5ftgihhq.irc PRIVMSG @#ops :deploy key rotated?"
    ))
    .await;
    let (audience, _) = next_restricted_message(&mut events).await;
    assert_eq!(audience.symbol(), '@');

    let reply = client
        .privmsg("#ops", "rotated at 18:00")
        .tag("+draft/reply", ID);
    assert!(widens_ops(&reply.await));
    let reaction = Line::tagmsg("#ops")
        .with_tag("+draft/react", "👀")
        .with_tag("+draft/reply", ID);
    assert!(widens_ops(&client.send_raw(reaction).await));
    let public_redact = Line::new("REDACT").with_params(["#ops", ID, "wrong channel"]);
    assert!(widens_ops(&client.request_raw(public_redact).await));

    client
        .send_raw(Line::new("PING").param("after-refusals"))
        .await
        .unwrap();
    assert_eq!(wire.next_sent().await, "PING after-refusals");
}

async fn announce_the_next_command(client: &papo::Client, events: &mut papo::Events) {
    loop {
        match events.next().await.map(|(event, _)| event) {
            Some(papo::Event::Message(message)) if message.text.starts_with('!') => {
                let announcement = format!("handled {}", message.text);
                client.privmsg("#ops", announcement).await.unwrap();
                return;
            }
            Some(papo::Event::Disconnected(disconnected)) => panic!("disconnected: {disconnected}"),
            Some(_) => {}
            None => panic!("driver stopped while waiting for a message"),
        }
    }
}

#[tokio::test]
async fn an_app_without_the_restricted_arm_never_answers_an_ops_only_line() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://hostile.offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["message-tags"]).await;
    wire.server(":offline.test 005 papo STATUSMSG=~&@%+ :are supported by this server")
        .await;
    wire.server(format!(
        "@time=2026-11-15T00:13:53.319Z;msgid={ID} :oppy!~u@87gpv5ftgihhq.irc PRIVMSG @#ops :!deploy key rotated?"
    ))
    .await;
    wire.server(
        "@time=2026-11-15T00:13:55.102Z;msgid=p3k8ri7wq2tcxe5jhb9mszd4ya :oppy!~u@87gpv5ftgihhq.irc PRIVMSG #ops :!ping",
    )
    .await;

    announce_the_next_command(&client, &mut events).await;
    assert_eq!(wire.next_sent().await, "PRIVMSG #ops :handled !ping");
}
