# Ping bot

This is the smallest complete bot: it connects, joins `#papo`, and answers each live `!ping`
with a threaded `pong` reply. A failed reply is logged and the bot keeps running.
Private replies are refused if client state proves that another user took the sender's nickname.

Pass an `irc://` or `ircs://` server URL as the first argument. The nickname comes from
`IRC_NICK` and defaults to `papo-ping`.

```sh
IRC_NICK=papo-ping cargo run -p papo-ping-bot -- ircs://irc.example.net
```
