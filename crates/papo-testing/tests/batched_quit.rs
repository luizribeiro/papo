//! A live QUIT in a batch ends replies to its sender.

mod support {
    pub mod direct;
}

use std::time::Duration;

use papo::{Client, Error, Event, Events, RateLimit, Reconnect, event::Message};
use papo_testing::{Wire, wire};
use support::direct::{connected, next_active_event};

const SPLIT: &str = ":offline.test BATCH +split netsplit hub.example leaf.example";
const QUIT: &str = "@batch=split :alice!~alice@cloak-7f3a.example QUIT :hub.example leaf.example";

struct Review {
    client: Client,
    events: Events,
    wire: Wire,
    message: Message,
}

impl Review {
    async fn start(accounts: bool) -> Self {
        let (client, mut events, mut wire) = wire(
            Client::builder("irc://offline.test", "papo")
                .rate_limit(RateLimit::unlimited())
                .reconnect(Reconnect::never())
                .event_capacity(10_010),
        );
        let mut caps = vec!["batch", "message-tags"];
        if accounts {
            caps.extend(["account-notify", "account-tag", "extended-join"]);
        }
        wire.register(&caps).await;
        connected(&mut events).await;
        wire.server(":papo!~u@papo.irc JOIN #review").await;
        if accounts {
            wire.server(
                "@account=alice :alice!~alice@cloak-7f3a.example JOIN #review alice :Alice",
            )
            .await;
        } else {
            wire.server(":alice!~alice@cloak-7f3a.example JOIN #review")
                .await;
        }
        let tags = if accounts {
            "msgid=ErgoDm1;account=alice"
        } else {
            "msgid=ErgoDm1"
        };
        wire.server(format!("@{tags} :alice!~alice@cloak-7f3a.example PRIVMSG papo :can you send me the release notes?")).await;
        loop {
            if let Event::Message(message) = next_active_event(&mut events).await {
                return Self {
                    client,
                    events,
                    wire,
                    message,
                };
            }
        }
    }

    async fn sync(&mut self) -> bool {
        self.wire
            .server(":offline.test NOTICE papo :*** end of test burst")
            .await;
        let mut abandoned = false;
        loop {
            match next_active_event(&mut self.events).await {
                Event::ServerNotice(notice) if notice.text == "*** end of test burst" => {
                    return abandoned;
                }
                Event::Warning(papo::event::Warning::BatchAbandoned { .. }) => abandoned = true,
                _ => {}
            }
        }
    }

    async fn start_split(&mut self) {
        self.wire.server(SPLIT).await;
        self.wire.server(QUIT).await;
    }

    async fn overflow_split(&mut self) {
        self.start_split().await;
        for index in 0..10_000 {
            self.wire.server(format!(
                "@batch=split :guest{index}!~u@leaf-{index}.example QUIT :hub.example leaf.example"
            )).await;
        }
        self.wire.server(":offline.test BATCH -split").await;
    }

    async fn assert_reply_refused(&mut self) {
        let result = self
            .client
            .reply(&self.message, "the release notes are ready")
            .await;
        assert!(matches!(result, Err(Error::TargetGone)), "{result:?}");
        assert!(
            tokio::time::timeout(Duration::from_millis(50), self.wire.next_sent())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn quit_in_a_batch_of_unknown_type_ends_the_reference() {
    let mut review = Review::start(true).await;
    for line in [
        ":offline.test BATCH +x7 example.org/maintenance",
        "@batch=x7;account=alice :alice!~alice@cloak-7f3a.example QUIT :Quit: bye",
        ":offline.test BATCH -x7",
    ] {
        review.wire.server(line).await;
    }
    review.sync().await;
    review.assert_reply_refused().await;
}

#[tokio::test]
async fn quit_in_an_oversized_netsplit_ends_the_reference() {
    let mut review = Review::start(true).await;
    review.overflow_split().await;
    assert!(review.sync().await);
    review.assert_reply_refused().await;
}

#[tokio::test]
async fn quit_in_an_oversized_netsplit_then_a_new_holder_joins() {
    let mut review = Review::start(false).await;
    review.overflow_split().await;
    review
        .wire
        .server(":alice!~mallory@tor-exit.example JOIN #review")
        .await;
    review
        .wire
        .server(":alice!~mallory@tor-exit.example PRIVMSG #review :hi all")
        .await;
    assert!(review.sync().await);
    review.assert_reply_refused().await;
}
