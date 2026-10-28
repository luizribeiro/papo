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
