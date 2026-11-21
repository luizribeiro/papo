//! Guarded sends across a bouncer upstream session.

mod support {
    pub mod direct;
    pub mod peer_pin;
}

use papo::{Error, Event, Reconnect};
use papo_testing::wire;
use support::{
    direct::{connected, next_active_event},
    peer_pin::join,
};

#[tokio::test]
async fn bouncer_upstream_drop_rejects_the_old_target() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("ircs://bouncer.example", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    wire.server(":bouncer.papo.test 005 papo BOUNCER_NETID=1 CASEMAPPING=ascii :are supported")
        .await;
    while !matches!(
        next_active_event(&mut events).await,
        Event::CasemappingChanged(_)
    ) {}
    join(
        &mut events,
        &mut wire,
        ":papo!~u@client.example JOIN #review",
    )
    .await;
    wire.server(":alice!alice-old@cloak.example JOIN #review")
        .await;
    let alice = loop {
        if let Event::Joined(joined) = next_active_event(&mut events).await {
            break joined.actor;
        }
    };
    wire.server(":BouncerServ!BouncerServ@BouncerServ NOTICE papo :disconnected from ergo")
        .await;
    while !matches!(next_active_event(&mut events).await, Event::Notice(_)) {}
    assert!(matches!(
        client.privmsg(&alice, "release approved").await,
        Err(Error::TargetGone)
    ));

    wire.server(":BouncerServ!BouncerServ@BouncerServ NOTICE papo :connected to ergo")
        .await;
    while !matches!(next_active_event(&mut events).await, Event::Notice(_)) {}
    join(
        &mut events,
        &mut wire,
        ":papo!~u@new-client.example JOIN #review",
    )
    .await;
    wire.server(":bouncer.papo.test 353 * = #review :@alice papo")
        .await;
    wire.server(":bouncer.papo.test 366 * #review :End of /NAMES list")
        .await;
    while !matches!(
        next_active_event(&mut events).await,
        Event::ChannelSynced(_)
    ) {}

    assert!(matches!(
        client.privmsg(&alice, "release approved").await,
        Err(Error::TargetGone)
    ));
}
