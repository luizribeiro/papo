# papo

`papo` is an ergonomic Rust library for IRC bots and clients on modern servers. It provides
typed asynchronous APIs, broad IRCv3 support with Ergo as its primary server, deterministic
sans-I/O protocol handling, and tokio-based operation on native and WASI targets.

The project is early in development; the API is not yet stable.

- `papo-proto` parses and builds IRC protocol data without I/O.
- `papo-core` implements deterministic sans-I/O IRC session behavior.
- `papo` provides the tokio driver, transports, client handles, and event stream.
- [`papo-bot`](crates/papo-bot) is an experimental, unpublished command-helper crate whose API
  will change.

## Quick start

```rust
use papo::{Client, Event};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (client, mut events) = Client::builder("ircs://irc.example.net")
        .nick("papo-bot")
        .autojoin(["#papo"])
        .connect()
        .await?;

    while let Some(event) = events.next().await {
        if let Event::Message(message) = event
            && message.is_live()
            && message.text == "!ping"
            && let Err(error) = client.reply(&message, "pong").await
        {
            eprintln!("could not reply: {error}");
        }
    }
    Ok(())
}
```

See the [ping bot](examples/ping-bot) for the complete example and its live-server test.

SASL, server passwords, WEBIRC, and account registration require `ircs://` or `wss://` by default.
Apps with another trusted transport layer must use `Builder::allow_insecure_auth(true)`.

## Capability implementation map

<!-- caps:start -->
| Capability | Aliases | Requested | Features | Code |
|---|---|---|---|---|
| `cap-notify` | — | default | — | `caps` |
| `message-tags` | — | default | client-only message tags, replies, reactions, typing notifications | `features`, `replies` |
| `server-time` | — | default | — | `state::read_marker` |
| `batch` | — | default | — | `batch` |
| `draft/event-playback` | — | default | — | `batch` |
| `echo-message` | — | default | — | `outgoing`, `echo` |
| `account-tag` | — | default | account tracking | `state::presence` |
| `extended-join` | — | default | — | `state::membership` |
| `multi-prefix` | — | default | — | `state::membership` |
| `userhost-in-names` | — | default | — | `state::membership` |
| `chghost` | — | default | host tracking | `state::presence` |
| `setname` | — | default | real-name changes | `state::presence` |
| `away-notify` | — | default | away tracking | `state::presence` |
| `account-notify` | — | default | account tracking | `state::presence` |
| `extended-monitor` | — | default | — | `monitor` |
| `invite-notify` | — | default | invite tracking | `replies` |
| `standard-replies` | — | default | — | `refusal`, `replies` |
| `labeled-response` | — | default | labeled-response | `outgoing`, `labels` |
| `draft/multiline` | — | default | multiline messages | `outgoing`, `batch` |
| `draft/oper-tag` | — | default | — | `replies` |
| `draft/metadata-3` | draft/metadata-2 | default | metadata | `metadata`, `state::metadata` |
| `metadata-notify` | — | fallback | — | `metadata`, `state::metadata` |
| `sasl` | — | automatic | — | `sasl` |
| `sts` | — | never | — | `sts` |
| `ergo.chat/nope` | — | never | — | `caps` |
| `soju.im/account-required` | — | never | — | `caps` |
| `draft/account-registration` | account-registration | default | account creation | `account_registration` |
| `soju.im/bouncer-networks` | — | opt-in | bouncer networks | `bouncer` |
| `soju.im/bouncer-networks-notify` | — | opt-in | — | `bouncer` |
| `draft/chathistory` | chathistory | default | chathistory | `history`, `batch` |
| `draft/read-marker` | read-marker | default | read-marker | `history`, `state::read_marker` |
| `draft/persistence` | — | opt-in | persistence | `persistence`, `state::persistence` |
| `draft/channel-rename` | — | default | channel rename | `channel_rename`, `state::rename` |
| `draft/relaymsg` | — | opt-in | relaymsg | `relaymsg` |
| `draft/message-redaction` | — | default | draft/message-redaction | `outgoing`, `replies` |
| `draft/webpush` | soju.im/webpush | opt-in | webpush | `webpush` |
| `draft/pre-away` | — | default | — | `state` |
| `no-implicit-names` | draft/no-implicit-names | default | — | `state::membership` |
| `draft/whoami` | — | default | — | `state` |
| `draft/extended-isupport` | — | default | — | `isupport` |
<!-- caps:end -->

Use `papo_proto::strip_formatting` when IRC formatting controls should be removed while
preserving the message text.

## Running and shutting down

`Client::builder(...).connect()` needs a running tokio runtime on native targets. It spawns the
driver and waits for the first connection to finish. For explicit driver ownership, call
`build()`, spawn or otherwise poll the returned `Driver` first, and only then await client
actions. An action cannot complete while its driver is not being polled.

`Events::next()` is cancel-safe. Dropping a request future does not prove whether its command
reached the driver, while dropping the driver ends the event stream without a final
`Event::Disconnected`. It is safe to wrap `connect()` in a timeout; the builder also has a
`connect_timeout` setting.

The [graceful shutdown example](examples/graceful-shutdown) handles SIGINT and SIGTERM and awaits
`Client::quit()` before the process exits.

## WASI

`wasm32-wasip2` and `wasm32-wasip3` need `--cfg tokio_unstable`; this repository sets it for
WASI targets in `.cargo/config.toml`. Wasmtime needs `-S inherit-network -S
allow-ip-name-lookup` for outbound networking and name lookup, plus `-S p3` for wasip3.

## Browser workers

`wasm32-unknown-unknown` runs the high-level client in dedicated workers, shared workers, and
Node.js using browser WebSockets. Pass a `ws://` or `wss://` URL directly to `Client::builder`;
TCP endpoint constructors and the TLS and STS builder settings are not available. TLS is controlled
by the browser for `wss://`, and IRC STS does not apply to WebSocket connections.

Run all checks with:

```sh
nix develop -c pre-commit run --all-files
nix develop -c pre-commit run --all-files --hook-stage pre-push
```
