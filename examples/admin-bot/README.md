# Admin bot

This bot demonstrates account-based permissions. It gives channel operator status with
`!op <nick>`, but only when the sender is logged in to the account in `IRC_ADMIN`. Anyone else
gets `cannot op <nick>: permission denied`, and a failed `MODE` is reported the same way with its
reason. `IRC_ADMIN` is required, and `IRC_CHANNEL` defaults to `#papo-admin`.

Pass an `irc://` or `ircs://` server URL as the first argument. The nickname comes from
`IRC_NICK` and defaults to `papo-admin`. The bot itself does not need an account.

```sh
IRC_NICK=papo-admin IRC_ADMIN=alice \
  cargo run -p papo-admin-bot -- ircs://irc.example.net
```

Sample reply:

```text
cannot op bob: permission denied
```
