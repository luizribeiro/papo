//! Highest-layer regression for public sends that point at an ops-only line.

use papo::{InvalidInput, Line, Reconnect};
use papo_testing::wire;

const ID: &str = "z6apvbu245cpg8t9b48v5bdmqs";

async fn next_message(events: &mut papo::Events) -> papo::Message {
    loop {
        match events.next().await.map(|(event, _)| event) {
            Some(papo::Event::Message(message)) => return message,
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
    let message = next_message(&mut events).await;
    assert_eq!(message.status().map(papo::Prefix::symbol), Some('@'));

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
