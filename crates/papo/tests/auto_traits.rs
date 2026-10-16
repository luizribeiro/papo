#![cfg(not(target_family = "wasm"))]
//! Compile-time guards for the thread-safety contracts of public types and futures.

use std::{future::IntoFuture, time::SystemTime};

use futures_core::Stream;
use papo::{
    Client, Error, Event, Events, ListFilter, MessageEvent, NewAccount, PersistenceSetting,
    PushKeys,
};
use papo_proto::{Message, Typing};

fn assert_send<T: Send>(_: T) {}

fn assert_send_sync<T: Send + Sync>() {}

fn assert_clone<T: Clone>() {}

fn assert_event_stream<T: Stream<Item = Event>>() {}

fn assert_message_request_futures(client: &Client, message: &MessageEvent) {
    assert_send(client.react(message, "👍"));
    assert_send(client.unreact(message, "👍"));
    assert_send(message.reply("Acknowledged"));
}

#[test]
fn public_handles_events_and_errors_keep_their_auto_traits() {
    assert_send_sync::<Client>();
    assert_send_sync::<Events>();
    assert_send_sync::<Event>();
    assert_send_sync::<Error>();
    assert_clone::<Client>();
    assert_event_stream::<Events>();
}

#[test]
fn request_futures_are_send() {
    let (client, _events, _driver) = Client::builder("irc.example.net").build();

    assert_send(client.send(Message::privmsg("#papo", "hello from the trait test")));
    assert_send(client.raw(Message::new("WHOIS", ["emily"])));
    assert_send(client.raw_unlabeled(Message::new("PING", ["trait-check"])));
    assert_send(client.privmsg("#papo", "hello from the trait test"));
    assert_send(client.notice("emily", "maintenance soon"));
    assert_send(client.typing("#papo", Typing::Active));
    assert_send(client.redact("#papo", "RfcBTzyxY120004", None));
    assert_send(client.whois("emily"));
    assert_send(client.who("#papo"));
    assert_send(client.join("#papo"));
    assert_send(client.part("#papo", Some("tests complete")));
    assert_send(client.mode("#papo", "+t", [] as [&str; 0]));
    assert_send(client.topic("#papo", Some("Ergonomic IRC")));
    assert_send(client.list(ListFilter::default()));
    assert_send(client.quit(Some("trait check complete")));

    assert_send(client.history("#papo").latest(10).into_future());
    assert_send(client.history_targets(SystemTime::UNIX_EPOCH, SystemTime::now()));
    assert_send(client.mark_read("#papo", SystemTime::UNIX_EPOCH));
    assert_send(client.read_marker("#papo"));
    assert_send(
        client.register_account(
            NewAccount::new("correct horse battery staple")
                .account("alice")
                .email("alice@example.org"),
        ),
    );
    assert_send(client.verify("alice", "392814"));
    assert_send(client.persistence().get());
    assert_send(client.persistence().set(PersistenceSetting::On));
    assert_send(client.webpush().register(
        "https://push.example.net/message/ErgoTest/alice",
        PushKeys::new("AQIDBAUGBwgJCgsMDQ4P_w", "-vv8_f7_"),
    ));
    assert_send(
        client
            .webpush()
            .unregister("https://push.example.net/message/ErgoTest/alice"),
    );
    assert_send(client.metadata().get("alice", ["avatar"]));
    assert_send(client.metadata().list("alice"));
    assert_send(client.metadata().set("status", Some("testing")));
    assert_send(
        client
            .metadata()
            .set_on("alice", "timezone", Some("America/New_York")),
    );
    assert_send(client.metadata().clear());
    assert_send(client.metadata().subscribe(["avatar", "pronouns"]));
    assert_send(client.metadata().unsubscribe(["pronouns"]));
    assert_send(client.metadata().sync("alice"));
    assert_send(client.rename("#rust", "#rust-lang", "Project renamed"));
    assert_send(client.relay("#bridged", "matrix|alice", "Hello from Matrix"));

    _ = assert_message_request_futures;
}
