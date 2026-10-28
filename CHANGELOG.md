# Changelog

## 0.1.0 - Unreleased

The first release of papo provides an asynchronous IRC client for applications and bots, broad
IRCv3 support, deterministic protocol state, and native, WASI, and browser transports. `papo` is
the supported application API; `papo-core` and `papo-proto` are lockstep implementation crates
without independent stability guarantees.

### Migration from the 2026-10-24 API

This section covers breaking changes since `46c832d`. Papo had not been released at that point,
but applications built against that revision need the following updates.

#### Connecting and configuring

The nickname is now required at the construction boundary. Known capabilities use typed features,
while `request_caps` is reserved for capability names unknown to papo.

Before:

```rust,ignore
let builder = Client::builder("ircs://irc.example.net")
    .nick("alice")
    .without_caps(["draft/read-marker"])
    .request_caps(["draft/relaymsg"]);
```

After:

```rust,ignore
let builder = Client::builder("ircs://irc.example.net", "alice")
    .disable(Feature::ReadMarker)
    .enable(Feature::Relay)
    .request_caps(["vendor.example/network-state"]);
```

| Earlier API | 0.1.0 API |
|---|---|
| `Client::builder(endpoint)` plus optional `Builder::nick` | `Client::builder(endpoint, nick)`; there is no implicit `papo` nickname |
| `without_caps(names)` and known names in `request_caps` | `disable(Feature)` / `enable(Feature)`; `request_caps` rejects known, managed, and unavailable unstable names |
| `Endpoint::websocket`, separate URL constructors, and no `FromStr` | Parse every scheme with `"ircs://…".parse::<Endpoint>()`; `plaintext` and `tls` remain host/port conveniences |
| Many URL-shaped `InvalidInput` variants and an unexported network URL error | `InvalidInput::Url(InvalidUrl)` and `Endpoint::Err = InvalidUrl` |
| `Builder::device` | `Sasl::plain(...).device(...)` or another SASL mechanism builder |
| No SASL authorization identity | `Sasl::authzid(...)`; PLAIN transmits it too |
| Empty `away_on_connect("")` silently meant no away | Empty initial away text is `Error::InvalidInput`; omit the setting to stay present |
| `Builder::stream(stream)` inferred security from the URL | `stream` is explicitly plaintext; use `stream_with_security(stream, StreamSecurity::Encrypted)` for an encrypted custom transport |
| TLS certificate private key accepted as `Secret` | `tls_client_cert(certificate_pem, private_key_pem)` keeps byte-oriented key material private |
| `Secret` accepted bytes and `expose()` returned `Option<&str>` | `Secret` is text-only and `expose()` returns `&str`; arbitrary TLS bytes no longer pass through it |
| Public-field `WebIrc` literal | `WebIrc::new(gateway, password, ip).hostname(...).flag(...)` |
| `sts_store` required an owned, mutable store | `StsStore` methods take `&self`, the value must be `Send + Sync`, and `Arc<T>` can share one store across clients and application persistence |
| Bad TLS PEM appeared as a late `ConnectErrorKind::Other` | TLS configuration is validated eagerly as `Error::InvalidInput` |
| `Duration::MAX` and oversized queue settings could panic | Maximum durations disable the corresponding deadline; event capacity is safely clamped |
| `ConnectError` hid a login refusal reason and repeated it in `source()` | Sanitized server reasons are part of `Display` and are not duplicated in the source chain |
| `ServerError::command() -> &str` used `""` for absence | `ServerError::command() -> Option<&str>` |

Credentials configured with `PASS`, SASL, WEBIRC, OPER, account verification, or bouncer password
fields are refused on plaintext transports unless STS upgrades the connection. Tests and trusted
locally protected transports can opt in with `allow_insecure_auth(true)`. Server-provided display
text is sanitized before reaching events, errors, and disconnect reasons.

Unstable bouncer network construction also changed from a public configuration value to request
builders:

```rust,ignore
// Before
let id = client.bouncer().add(NetworkConfig::from_url(url)?).await?;
client.bouncer().change(&id, changed_config).await?;

// After
let id = client.bouncer().add(url).name("work").nick("alice").await?;
client.bouncer().change(&id).name("work-renamed").await?;
```

Only fields selected on `change` are sent. Text retained by builders now consistently accepts
`Into<String>` (including `String` without a forced borrow), while read-only arguments use the
sealed role traits described below.

#### Names, destinations, and guarded identities

`Chat` was removed. `Conversation` is now the single channel-or-private conversation type, but it
is deliberately not a send target: a conversation does not preserve an ops-only status prefix or
prove that a private nickname still belongs to the same user.

Before:

```rust,ignore
let conversation: Chat = message.conversation.clone();
client.privmsg(&conversation, "acknowledged").await?;
client.invite("alice", "#papo").await?;
client.mode("#papo").op("alice").await?;
```

After:

```rust,ignore
client.reply(&message, "acknowledged").await?;

let alice = client.direct("alice").await?;
client.privmsg(&alice, "private update").await?;
client.invite(&alice, "#papo").await?;
client.mode("#papo").op(&alice).await?;
```

`DirectRef` follows nickname changes, holds a `MONITOR` lease for users who share no channel, and
refuses with `Error::TargetGone` when the identity leaves or the nickname changes hands. Use
`direct_to(&message)` to capture the sender of a received private message. Clones share the lease.
Private `MessageRef` values and queued sends carry the same guard. `send_raw` remains the explicit
unguarded escape hatch.

| Earlier API | 0.1.0 API |
|---|---|
| `Chat::{Channel, Private}` plus `Conversation` | Only `Conversation`; event `conversation` fields use it |
| One `Target` role accepted nicks, masks, channels, conversations, and swapped arguments | Sealed `AsChannel`, `AsNick`, `AsAccount`, `AsMask`, and `AsConversation` roles reject category mistakes and foreign-client typed names |
| Nick text or `Nick` could be a private message target | Private sends require `DirectRef`, or a guarded `MessageRef`/`NoticeRef`; channels remain ordinary targets |
| `Conversation: Target` allowed `@#ops` replies to widen to `#ops` | Use `reply`, `react`, `redact`, or the event's `return_address()` so the original audience is retained |
| `return_address() -> String` | Message-like events return guarded `MessageRef`; notices return `NoticeRef`; server notice addresses are `Option<NoticeRef>` |
| Notices converted to `MessageRef` and could be passed to `reply` or `react` | `NoticeRef` can be sent with `notice`, but cannot be used for message replies or reactions |
| `invite`, `op`, `halfop`, `voice`, and arbitrary prefix modes accepted an unguarded nick | Privilege grants require `&DirectRef`; `SetModes::prefix(mode, &direct)` handles advertised custom prefix modes |
| `kickban(channel, nick, reason, ban)` | `kickban(channel, nick, ban, reason)`; `ban` is typed as `BanTarget` |
| Channel receipts were only text | Receipt-derived sends verify that a parted or recreated channel is still the same membership identity |
| `HistoryReference: From<String/&str>` guessed that text was a message ID | Construct `HistoryReference` explicitly from a `MessageId` or time |

Methods that operate on a channel (`join`, `part`, topic, channel rename, list modes) now use
`AsChannel`; nickname queries and changes use `AsNick`; `who` uses `AsMask`; metadata, history,
read markers, typing lookup, and mode targets use `AsConversation`. Metadata targets no longer use
the send-only `Target`. `relay` likewise uses role-specific source and destination arguments.

Name construction moved out of `State` and into rules that came from the server:

```rust,ignore
// Before
let nick = client.state().nickname("Alice");
if client.state().is_valid_nick(candidate) { /* ... */ }

// After
let state = client.state();
let rules = state.name_rules().ok_or("waiting for server ISUPPORT")?;
let nick = rules.nick("Alice");
match rules.check_nick(candidate) {
    NameCheck::Valid => {}
    NameCheck::Invalid(reason) => eprintln!("invalid nickname: {reason}"),
    NameCheck::Unknown => eprintln!("the server did not advertise this limit"),
}
```

`State::name_rules()` is `None` until `005` supplies the server rules, unless
`Builder::casemapping` seeds them. Requests that need name resolution return
`InvalidInput::NameRulesUnknown` before that point. `State::{nickname, channel_name, account_name,
mask, conversation, parse_target, parse_extban, is_valid_nick, is_valid_channel}` were removed;
their `NameRules` replacements are `nick`, `channel`, `account`, `mask`, `conversation`, `target`,
`parse_extban`, `check_nick`, and `check_channel`.

Names now remember both their producing client and casemapping, and equality, ordering, and hashing
include those facts. Text comparisons still apply the name's fold. The public
`Casemapping::{nick, channel, account, mask}` constructors and name `check()` methods were hidden.
`Casemapping` now implements `FromStr` and `Display`, with `as_str()` for its wire token.
`isupport::Extban` was renamed `ExtbanSyntax`; `Isupport::casemapping()` is optional, and status
prefixes from ISUPPORT, WHO/WHOIS, members, and event audiences use the shared `Prefix` and `Rank`
types rather than chars, indexes, or private prefix structs.
