# Changelog

## 0.1.0 - Unreleased

The first release of papo provides an asynchronous IRC client for applications and bots, broad
IRCv3 support, deterministic protocol state, and native, WASI, and browser transports. `papo` is
the supported application API; see the [IRCv3 support table](docs/ircv3-support.md) for the full
protocol surface. `papo-testing` is published and version-locked to `papo`, with no stability
promise even between patch releases. `papo-core` and `papo-proto` are internal lockstep
implementation crates without independent stability guarantees.

The default `tls-ring` Cargo feature provides TLS. `tls-aws-lc` selects the alternative AWS-LC
provider, `websocket` adds native and WASI WebSocket transports, `precis` adds Unicode nickname
and account comparison, and `unstable` exposes experimental extension APIs without semantic
versioning guarantees.

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
    .disable(Feature::ReadMarkers)
    .enable(Feature::Metadata)
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
| `Duration::MAX` and oversized queue settings could panic | Maximum durations disable the corresponding deadline; event capacity above the runtime bound is clamped, while zero is `InvalidInput::ZeroEventCapacity` |
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
`DirectRef::nick()` returns the current nickname, or the last known nickname after departure.
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
    _ => {}
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

#### Events, actors, and message references

Line events now use a consistent envelope and actor model. Match payloads directly; the message,
notice, server-notice, history, and auto-join-failure variants are no longer boxed or inline-field
special cases.

Before:

```rust,ignore
if let Event::OwnMessage(message) | Event::Message(message) = event {
    println!("{}: {}", message.actor, message.text);
    let reply_to = message.reply_to.clone();
}
if let Event::AutoJoinFailed { channel, error } = event { /* ... */ }
```

After:

```rust,ignore
if let Event::Message(message) = event {
    println!("{}: {}", message.actor.nick(), message.text);
    let reply_to = message.in_reply_to.as_ref();
    let own = message.authorship == Authorship::Own;
}
if let Event::AutoJoinFailed(failure) = event { /* ... */ }
```

| Earlier API | 0.1.0 API |
|---|---|
| Boxed `Event::{Message, Notice, ServerNotice, History}` payloads | Unboxed payloads; the Box-specific `MessageRef` conversions were removed |
| Inline-field `Event::AutoJoinFailed { channel, error }` | `Event::AutoJoinFailed(event::AutoJoinFailed)` and `as_auto_join_failed()` |
| `Event::OwnMessage` plus incoming variants | `Event::OwnMessage` was removed; all own lines use their normal variant with `Authorship::Own` |
| Own sends could also arrive as message events | A correlated send resolves to `Sent`; applications that log chat insert their own sends from that value |
| `Source` plus repeated `nick`, `account`, `oper`, and `is_bot` fields | `Source::{User(UserSource), Server(ServerName)}`; user facts live on `UserSource` accessors |
| `Message::reply_to` / `Notice::reply_to` | `in_reply_to` |
| `Reacted::message` and `removed: bool` | `reacts_to` and `ReactionAction::{Added, Removed}` |
| `Redacted::message` | `redacts` |
| `NoticeKind` duplicated message kinds | Notices use the shared `MessageKind`; `NoticeKind` was removed |
| `Vec<char>` status prefixes | `Option<Prefix>` preserves the lowest-ranked addressed prefix |
| `ServerNoticeTarget` | `Audience::{Me, Channel, Broadcast}` plus `Broadcast::{Everyone, Mask}` |
| Server-wide CTCP and tag messages were folded into unrelated events | `Event::BroadcastCtcp` and `Event::BroadcastTagMsg` carry explicit broadcast audiences |
| `Event::Other(Line)` | `Event::Other(event::Unparsed)` preserves the line plus msgid and timestamps and dereferences to `Line` |
| `Event::msgid()` cloned an identifier | It returns `Option<&MessageId>` |
| `HistoryPage::messages()` silently omitted notices | Use the page iterator for every item or `chat_messages()` when only `Message` values are wanted |

`Message`, `Notice`, `TagMsg`, `ServerNotice`, `Invited`, and `ChannelRenamed` now retain the full
available line envelope (`msgid`, `server_time`, and `received_at`). Derived account, away, host,
and typing changes retain the triggering time but intentionally do not duplicate its message ID.
`Event::{msgid, server_time, received_at}` cover every applicable variant.

User lifecycle payloads were normalized as follows:

| Earlier fields | 0.1.0 fields |
|---|---|
| `NickChanged { old, actor, account, ... }` | `actor: UserSource` describes the old identity, `new` is the new nick, and `channels` lists shared channels |
| `AwayChanged { nick, away, actor }` | `actor: UserSource`, `new: Away`, `authorship`, and timestamps |
| `AccountChanged { nick, account, actor }` | `actor: UserSource`, `new: Account`, `authorship`, and timestamps |
| `HostChanged { nick, username, host, actor }` | `actor: UserSource`, `new_username`, `new_host`, `authorship`, and timestamps |
| `RealnameChanged { nick, realname, actor }` | `actor: UserSource`, `new`, and `authorship` |
| `MonitorOnline { nick, username, host }` / `MonitorOffline { nick }` | `subject: UserSource` / `subject: Nick` |
| `ModeChanged::subject_authorship` | `ModeChanged::about(&change)` computes authorship for each member mode change |
| String netsplit server names | `SplitServers` contains `ServerName` values |

`Whois::account` is now `Account`, and bot flags in `Whois` and `WhoEntry` are `Option<bool>` so
unknown is not reported as false. WHO and WHOIS membership prefixes are `Prefix`; their channel
rows provide `rank()`. `MetadataChanged::value` is `Option<MetadataValue>` rather than separate
text and visibility fields.

#### Requests, results, and state views

Sent-message confirmation is explicit and retains every logical line's exact text:

```rust,ignore
// Before
if let Some(id) = sent.msgid { /* first line only */ }
for id in sent.line_msgids { /* text unavailable */ }

// After
match &sent.outcome {
    SentOutcome::Confirmed { lines, server_time, .. } => {
        for (id, text) in lines { /* store the confirmed line */ }
    }
    SentOutcome::Unconfirmed => { /* no usable message IDs */ }
    _ => {}
}
```

`Sent::{msgid, line_msgids, server_time}` were removed. `Sent::status` is `Option<Prefix>`, and
`return_address()` returns a guarded `MessageRef`. Reacting or redacting an unconfirmed send fails
because it has no server message ID. Redacting a confirmed split send processes every line in send
order as one driver-owned composite operation.

| Earlier API | 0.1.0 API |
|---|---|
| `client.list(ListFilter::default().channels(...))` | `client.list().channels(...).await`; omit `channels` for all visible channels |
| `Metadata::get(target, keys)` returned the whole cache | It returns exactly each requested key mapped to `Option<MetadataValue>` |
| Mixed `AsRef<str>` / `Into<String>` reason parameters and mandatory part text | Stored reasons use `Into<String>` consistently; `""` means no reason |
| `ModesApplied { applied, refused }` could report a partial batch as complete | `unconfirmed: Vec<(ModeChange, Error)>` and `is_complete()` expose missing confirmations |
| Dropping mode, kickban, or multiline-redact futures could stop midway | Once accepted by the driver, composite actions continue even if the caller drops its future |
| `State::read_marker() -> Option<SystemTime>` conflated unknown and explicitly unset | `Option<ReadMarker>` distinguishes unknown, `Unset`, and `ReadUntil(time)` |
| `State` implemented a partly deep `Clone` | Keep and clone the `Arc<State>` returned by `Client::state`; channel snapshots share unchanged storage |
| `State::metadata() -> &HashMap` exposed storage and nondeterministic order | `MetadataView` provides `get` and an exact-size iterator in metadata-key order |
| `State::typing(&Conversation)` tied the iterator to the argument borrow | `typing(impl AsConversation) -> Typers<'_>` owns its resolved lookup key |
| `Typer::return_address() -> String` lost private-recipient guards | It returns a guarded `MessageRef` while preserving status-prefixed audiences |
| State collection iterators hid known lengths | Channels, monitored nicks, metadata subscriptions, metadata entries, and tags expose exact sizes where available |
| `Tags` required `.iter()` and had no length | `&Tags` implements `IntoIterator`; `len()` and `TagsIter` are public |
| `Isupport::targmax()` required allocating and scanning all entries | `targmax_for(command)` performs a direct lookup |

`Account` and `Away` provide variant accessors. Name types and `MessageId` compare with text in
either operand order. `Error` gains `From<ServerError>`, `From<StateLimit>`, and
`From<Unsupported>` conversions. Input errors are now specific (`InvalidName`, `WireMessage`,
`InvalidMode`, `InvalidMetadataKey`, and related variants) rather than the catch-all
`InvalidMessage(String)`.

`client.direct` can return `Error::MonitorFull` when an unshared private recipient cannot reserve
a server `MONITOR` slot. `Error::Unconfirmed` reports a state-changing request whose wire outcome
could not be proven.

All request futures are lazy until first poll. After polling, dropping the future abandons the
wait but does not cancel driver work. `Error::Timeout` means every line was written and the reply
was not observed; only read-only queries are safe to retry automatically. Sends, KICK, MODE,
redaction, kickban, and every other state-changing request can already have taken effect.

#### Event lag and shutdown

Event delivery no longer stops socket reads or disconnects a send-only client when its consumer is
slow. The bounded queue discards state-backed events first, then other non-message events, and
message-like events last. The next item is one `Event::Lagged` for the episode:

```rust,ignore
match event {
    Event::Lagged(lag) => {
        let current = client.state();
        for gap in lag.gaps {
            // Query client.history(&gap.conversation) when history is available.
        }
    }
    event => handle(event),
}
```

`Lagged::dropped` counts all discarded events. Each bounded `MessageGap` identifies its
conversation, dropped count, and first/last known msgid and server time. Re-read `Client::state()`
for membership, modes, topics, presence, metadata, read markers, and typing; papo does not fill
message gaps automatically. `Builder::event_capacity` defaults to 1,024 events and should cover
the largest expected burst while the event loop is busy, not merely the average rate.

The connection now stays alive while either a `Client` clone or its `Events` stream exists.
Dropping all clients does not end an observe-only event loop, and dropping only `Events` does not
stop a client that still sends. Dropping the final handle asks the driver to send `QUIT`, but
cannot await delivery or acknowledgement. For intentional shutdown:

```rust,ignore
client.quit("shutting down").await?;
while events.next().await.is_some() {}
```

Dropping the `Driver` still ends the stream without a final `Event::Disconnected`.

#### Other observable corrections

- Incoming text, server reasons, malformed lines, and URL failures use bounded, terminal-safe
  display text. Channel keys and other credentials remain redacted in state, events, raw request
  responses, errors, and `Debug` output.
- Server-driven LIST/WHO/WHOIS/NAMES replies, read markers, monitored identities, event loss
  accounting, open batches, and other retained collections are bounded. Overflow returns a typed
  `StateLimit` error or warning instead of growing without limit or silently truncating a
  successful result. `Warning::BatchAbandoned` reports
  `BatchAbandonReason::{OpenBatchLimit, LineLimit, ByteLimit, Timeout}`.
- `Warning::Utf8CasemappingUnsupported` reports a server that selects RFC 8265 when the `precis`
  feature is absent. Name checks use only limits actually advertised by the server.
- `Warning::ChannelRecreated` reports a rejoined channel whose creation time changed. Typing,
  read markers, and other channel state move on a legitimate rename and are cleared for a new
  channel identity.
- A nickname lifecycle now clears metadata and all other per-user facts before a new user can
  inherit them. Own-user state is one coherent row and resists hostile nickname collisions.
- Outgoing text that cannot fit even one UTF-8 scalar within the server's advertised multiline
  limit returns an error rather than silently dropping text.
- Length-based splits in outgoing `draft/multiline` messages concatenate without introducing
  line breaks; confirmed text exactly preserves user-authored spaces and newlines.
- Multiline messages replayed inside chat history, event playback, and labeled-response batches
  retain their exact text (including blank lines), authorship, server time, message ID, account,
  and other opening tags.
- Reconnect catch-up deduplicates every live event by msgid, including messages, and own sends
  consume their correlated echo before application delivery.
- `Typing::Done` clears the active-send throttle, so a following active notification is not
  incorrectly suppressed.
- `ModeChange` and other externally constructed protocol values are non-exhaustive where future
  server vocabulary can add cases. `Typer` exposes `nick()`, `is_paused()`, `status()`, and its
  audience-preserving return address rather than mutable public fields; the impossible retained
  `Done` state and redundant conversation field are gone.
- Channel mode lookup has one `ModeValue` representation. `User::is_bot` is optional where the
  server has not supplied the fact. Account and away state preserve unknown values instead of
  guessing.
- The duplicate `Isupport::filehost()` accessor was removed; use `State::filehost()` (requires the
  `unstable` feature) for the validated upload endpoint. Raw advertised values remain available
  through `State::isupport()`.
