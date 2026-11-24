//! A bare presence row does not attest who holds the nick.

mod support {
    pub mod direct;
    pub mod message_peer;
}

use support::direct::{join, next_active_event};
use support::message_peer::{FIRST, assert_refused, message, read, start};

/// Joins `#review`, where the server lists a bare `alice` (no `userhost-in-names`, as through
/// soju), and returns after papo's own `WHO #review` is on the wire.
async fn join_beside_a_bare_alice(events: &mut papo::Events, wire: &mut papo_testing::Wire) {
    join(events, wire, ":papo!~u@papo.irc JOIN #review").await;
    wire.server(":offline.test 353 papo = #review :@alice papo")
        .await;
    wire.server(":offline.test 366 papo #review :End of /NAMES list.")
        .await;
    loop {
        if let papo::Event::ChannelSynced(_) = next_active_event(events).await {
            break;
        }
    }
    assert_eq!(wire.next_sent().await, "MODE #review");
    assert_eq!(wire.next_sent().await, "WHO #review");
}

#[tokio::test]
async fn who_row_with_another_host_after_a_bare_names_row_ends_the_reference() {
    let (client, mut events, mut wire) = start(&["message-tags"]).await;
    let first = message(&mut events, &mut wire, &[FIRST]).await;
    join_beside_a_bare_alice(&mut events, &mut wire).await;
    read(
        &mut events,
        &mut wire,
        &[
            ":offline.test 352 papo #review ~mallory tor-exit.example offline.test alice H@ :0 Mallory",
            ":offline.test 352 papo #review ~u papo.irc offline.test papo H :0 Papo",
            ":offline.test 315 papo #review :End of WHO list",
        ],
    )
    .await;
    assert_refused(client, &mut wire, first).await;
}

#[tokio::test]
async fn channel_message_from_another_host_after_a_bare_names_row_ends_the_reference() {
    let (client, mut events, mut wire) = start(&["message-tags"]).await;
    let first = message(&mut events, &mut wire, &[FIRST]).await;
    join_beside_a_bare_alice(&mut events, &mut wire).await;
    _ = message(
        &mut events,
        &mut wire,
        &[":alice!~mallory@tor-exit.example PRIVMSG #review :hi all"],
    )
    .await;
    assert_refused(client, &mut wire, first).await;
}
