# papo-testing

Unpublished, unstable, offline-first test support for applications built with
`papo`. Live-server fixtures belong to the conformance suite rather than this
crate's default surface.

## Test a bot without a server

`wire` runs a real client over an in-memory connection. Tests provide server
lines and inspect the lines that the client sends, while Tokio's paused clock
keeps timers deterministic.

```rust
use papo_testing::wire;

#[tokio::test(start_paused = true)]
async fn ping() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-bot")
            .reconnect(papo::Reconnect::never()),
    );
    wire.register(&["message-tags"]).await;
    wire.server(":alice!~alice@cloak.example PRIVMSG papo-bot :!ping")
        .await;

    while let Some(event) = events.next().await {
        let Some(message) = event.as_message() else { continue };
        if message.text == "!ping" {
            client.reply(message, "pong").await.unwrap();
            break;
        }
    }

    assert_eq!(wire.next_sent().await, "PRIVMSG alice pong");
}
```
