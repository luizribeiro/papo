# Admin bot

This example uses the unreleased, unstable `papo-bot` crate, which is not available on crates.io
and whose API will change.

This bot demonstrates command parsing, commands addressed by mention, and account-based
permissions. It answers `!ping` and allows the account in `IRC_ADMIN` to use `!op <nick>`.
`IRC_CHANNEL` defaults to `#papo-admin`.

Pass an `irc://` or `ircs://` server URL as the first argument. The nickname comes from
`IRC_NICK` and defaults to `papo-admin`. The bot itself does not need an account.

```sh
IRC_NICK=papo-admin IRC_ADMIN=alice \
  cargo run -p papo-admin-bot -- ircs://irc.example.net
```

Sample replies:

```text
pong
cannot op bob: permission denied
```
