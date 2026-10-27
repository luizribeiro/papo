# papo-testing

Offline wire fixtures and value builders for applications built with `papo`.

`papo-testing` has no API stability promise, including between patch releases.
Its version always matches `papo`'s version; pin both crates exactly so their
types and hidden construction hooks stay in sync:

```toml
[dev-dependencies]
papo = "=0.1.0"
papo-testing = "=0.1.0"
```

The supported surface is `wire()` plus builders for representative
`papo` values. Live-server fixtures belong to papo's conformance suite.

## Test a bot without a server

`wire()` runs a real client over an in-memory connection. Tests provide server
lines and inspect the lines that the client sends, while Tokio's paused clock
keeps timers deterministic. It works on native Rust, WASI, and browser wasm.

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
