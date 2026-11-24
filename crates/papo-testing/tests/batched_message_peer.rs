//! Identity evidence received before a batch is closed.

mod support {
    pub mod direct;
    pub mod message_peer;
}

use support::message_peer::{FIRST, assert_refused, message, read, start};

/// The batch specification has clients process the members of a batch whose type they do not
/// know as ordinary lines.
#[tokio::test]
async fn message_in_a_batch_of_unknown_type_from_another_host_ends_the_reference() {
    let (client, mut events, mut wire) = start(&["batch", "message-tags"]).await;
    let first = message(&mut events, &mut wire, &[FIRST]).await;
    read(
        &mut events,
        &mut wire,
        &[
            ":offline.test BATCH +x7 example.org/maintenance",
            "@batch=x7;msgid=ErgoDm2 :alice!~mallory@tor-exit.example PRIVMSG papo :hello?",
        ],
    )
    .await;
    assert_refused(client, &mut wire, first).await;
}
