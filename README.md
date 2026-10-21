# papo

`papo` is an ergonomic Rust library for IRC bots and clients on modern servers. It provides
typed asynchronous APIs, broad IRCv3 support with Ergo as its primary server, deterministic
sans-I/O protocol handling, and tokio-based operation on native and WASI targets.

The project is early in development; the API is not yet stable.

- `papo-proto` parses and builds IRC protocol data without I/O.
- `papo-core` implements deterministic sans-I/O IRC session behavior.
- `papo` provides the tokio driver, transports, client handles, and event stream.

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
| `invite-notify` | — | default | — | `replies` |
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
| `soju.im/bouncer-networks` | — | default | bouncer networks | `bouncer` |
| `soju.im/bouncer-networks-notify` | — | default | — | `bouncer` |
| `chathistory` | draft/chathistory | default | chathistory | `history`, `batch` |
| `draft/read-marker` | read-marker | default | read-marker | `history`, `state::read_marker` |
| `draft/persistence` | — | default | persistence | `persistence`, `state::persistence` |
| `draft/channel-rename` | — | default | channel rename | `channel_rename`, `state::rename` |
| `draft/relaymsg` | — | default | relaymsg | `relaymsg` |
| `draft/message-redaction` | — | default | draft/message-redaction | `outgoing`, `replies` |
| `draft/webpush` | soju.im/webpush | default | webpush | `webpush` |
| `draft/pre-away` | — | default | — | `state` |
| `no-implicit-names` | draft/no-implicit-names | default | — | `state::membership` |
| `draft/whoami` | — | default | — | `state` |
| `draft/extended-isupport` | — | default | — | `isupport` |
<!-- caps:end -->

Use `papo_proto::strip_formatting` when IRC formatting controls should be removed while
preserving the message text.

## WASI

`wasm32-wasip2` and `wasm32-wasip3` need `--cfg tokio_unstable`; this repository sets it for
WASI targets in `.cargo/config.toml`. Wasmtime needs `-S inherit-network -S
allow-ip-name-lookup` for outbound networking and name lookup, plus `-S p3` for wasip3.

## Browser workers

`wasm32-unknown-unknown` runs the high-level client in dedicated workers, shared workers, and
Node.js using browser WebSockets. Build with a `ws://` or `wss://` `Endpoint::websocket`; TCP
endpoint constructors and the TLS and STS builder settings are not available. TLS is controlled by
the browser for `wss://`, and IRC STS does not apply to WebSocket connections.

Run all checks with:

```sh
nix develop -c pre-commit run --all-files
nix develop -c pre-commit run --all-files --hook-stage pre-push
```
