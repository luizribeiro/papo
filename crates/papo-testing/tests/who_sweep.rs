//! Highest-layer regression for WHO rows that describe an earlier holder of a nick.

use papo::{Account, Event, Rank, Reconnect};
use papo_testing::wire;

async fn members_refreshed(events: &mut papo::Events) -> std::sync::Arc<papo::State> {
    loop {
        match events.next().await {
            Some((Event::MembersRefreshed(_), state)) => return state,
            Some((Event::Disconnected(disconnected), _)) => panic!("disconnected: {disconnected}"),
            Some(_) => {}
            None => panic!("driver stopped before the WHO sweep finished"),
        }
    }
}

#[tokio::test]
async fn a_nick_taker_never_inherits_the_previous_holders_account_or_rank() {
    let (_client, mut events, mut wire) = wire(
        papo::Client::builder("irc://hostile.offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    wire.server(":offline.test 005 papo WHOX PREFIX=(ov)@+ :are supported by this server")
        .await;
    wire.server(":papo!~papo@203.0.113.1 JOIN #lobby").await;
    wire.server(":offline.test 353 papo = #lobby :@alice papo")
        .await;
    wire.server(":offline.test 366 papo #lobby :End of NAMES list")
        .await;
    assert_eq!(wire.next_sent().await, "MODE #lobby");
    assert_eq!(wire.next_sent().await, "WHO #lobby %tcuhnfar,152");

    wire.server(":alice!~alice@staff.example QUIT :Quit: leaving")
        .await;
    wire.server(":alice!~mallory@198.51.100.23 JOIN #lobby")
        .await;
    wire.server(
        ":offline.test 354 papo 152 #lobby ~alice staff.example alice H@ alice :Alice Smith",
    )
    .await;
    wire.server(":offline.test 315 papo #lobby :End of WHO list")
        .await;

    let state = members_refreshed(&mut events).await;
    let taker = state.user("alice").unwrap().unwrap();
    assert_eq!(taker.username(), Some("~mallory"));
    assert!(!matches!(taker.account(), Account::LoggedIn(_)));
    let channel = state.__channel_local("#lobby").unwrap();
    let member = channel.member("alice").unwrap().unwrap();
    assert!(!member.has(Rank::Op));
}
