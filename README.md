# papo

`papo` is an ergonomic Rust library for IRC bots and clients on modern servers. It provides
typed asynchronous APIs, broad IRCv3 support with Ergo as its primary server, deterministic
sans-I/O protocol handling, and tokio-based operation on native and WASI targets.

The project is early in development; the API is not yet stable. `papo` is the library applications
should use. The published `papo-proto` and `papo-core` packages are internal implementation crates
required by `papo`; they are not intended for direct use and have no API stability guarantees.

## Quick start

```rust
use papo::{Client, Event};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (client, mut events) = Client::builder("ircs://irc.example.net", "papo-example")
        .autojoin(["#papo"])
        .connect()
        .await?;

    while let Some(event) = events.next().await {
        if let Event::Message(message) = event
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

String server URLs must start with `irc://`, `ircs://`, `ws://`, or `wss://`; bare hostnames are
rejected. Use `ircs://` for IRC over TLS.

SASL, WEBIRC, and account registration require `ircs://` or `wss://` by default. A server password
on plaintext TCP is sent only after a valid IRC STS upgrade. Apps with another trusted transport
layer must use `Builder::allow_insecure_auth(true)` to permit credentials without transport TLS.

Builder settings that depend on a capability fail connection setup when it is unavailable. This
includes account creation, metadata subscriptions, reconnect history, and selecting a bouncer
network. In contrast, `enable(Feature::X)` uses known capabilities only when offered;
`request_caps` is reserved for vendor capabilities that papo does not know.

Pass a received message or `Sent` value to reply, react, unreact, or redact. That preserves a
status-prefixed return address such as `@#ops`; the `Conversation` value identifies only the
`#ops` buffer and must not be reconstructed into a send target. For a private received line,
these methods return `Error::TargetGone` when current state proves that another user took the
sender's nickname. They still send when papo cannot identify the current holder, such as a direct
message from someone who shares no channel and supplied no account information.

Typed names, conversations, message references, and history cursors retain the `Client` that
produced them. The sealed `Target` trait lets send methods accept either plain text or these typed
values; a typed value from another client returns `InvalidInput::ForeignReference`. Consequently,
same-named channels on two clients compare unequal, while either still compares equal to the same
plain string. Converting a typed value to a string explicitly discards this protection.

papo answers private, live CTCP `PING` and `VERSION` queries by default; the VERSION text is
`papo <version>`. Use `CtcpReplies::default().ping(false)`, `.version("my-app 2.3")`, or
`.without_version()` to change either reply, and `CtcpReplies::off()` to answer nothing automatically.
Other queries are emitted as `Event::CtcpQuery` and can be answered with `Client::ctcp_reply`.
Replies arrive separately as `Event::CtcpReply`, so they cannot be passed to `ctcp_reply`. Replayed,
channel-targeted, and own-session queries are never answered automatically.

## Capability implementation map

<!-- caps:start -->
| Capability | Aliases | Requested | Features | Code |
|---|---|---|---|---|
| `cap-notify` | — | default | capability notifications | `caps` |
| `message-tags` | — | default | client-only message tags, replies, reactions, typing notifications | `features`, `replies` |
| `server-time` | — | default | server timestamps | `state::read_marker` |
| `batch` | — | default | message batches | `batch` |
| `draft/event-playback` | — | default | event playback | `batch` |
| `echo-message` | — | default | message echoes | `outgoing`, `echo` |
| `account-tag` | — | default | account tracking | `state::presence` |
| `extended-join` | — | default | extended join details | `state::membership` |
| `multi-prefix` | — | default | complete membership prefixes | `state::membership` |
| `userhost-in-names` | — | default | user and host fields in names replies | `state::membership` |
| `chghost` | — | default | host tracking | `state::presence` |
| `setname` | — | default | real-name changes | `state::presence` |
| `away-notify` | — | default | away tracking | `state::presence` |
| `account-notify` | — | default | account tracking | `state::presence` |
| `extended-monitor` | — | default | extended monitor replies | `monitor` |
| `invite-notify` | — | default | invite tracking | `replies` |
| `standard-replies` | — | default | structured replies | `refusal`, `replies` |
| `labeled-response` | — | default | labeled-response | `outgoing`, `labels` |
| `draft/multiline` | — | default | multiline messages | `outgoing`, `batch` |
| `draft/oper-tag` | — | default | operator tags | `replies` |
| `draft/metadata-3` | draft/metadata-2 | default | metadata | `metadata`, `state::metadata` |
| `metadata-notify` | — | fallback | — | `metadata`, `state::metadata` |
| `sasl` | — | automatic | SASL authentication | `sasl` |
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
| `draft/pre-away` | — | default | registration-time away state | `state` |
| `no-implicit-names` | draft/no-implicit-names | default | suppression of implicit names replies | `state::membership` |
| `draft/whoami` | — | default | connection identity discovery | `state` |
| `draft/extended-isupport` | — | default | extended ISUPPORT discovery | `isupport` |
<!-- caps:end -->

Use `papo::strip_formatting` when IRC formatting controls should be removed while preserving the
message text.

## Running and shutting down

`Client::builder(..., "papo-client").connect()` needs a running tokio runtime on native targets. It spawns the
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
