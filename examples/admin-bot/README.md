# Admin bot

This bot demonstrates account-based permissions. It gives channel operator status with
`!op <nick>`, when `Message::actor_account()` matches `IRC_ADMIN`. It grants operator status
with `client.set_modes(channel).op(nick)`. Anyone else gets `cannot op <nick>: permission denied`,
and a failed mode change is reported with its reason. `IRC_ADMIN` is required, and `IRC_CHANNEL` defaults to `#papo-admin`.

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
