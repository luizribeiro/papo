# Echo bot

This bot demonstrates three modern IRC message features. It replies with the text it receives,
preserves multi-line messages, and reacts with 👍 when it receives `!react`. Failed actions are
logged without stopping the bot.

Pass an `irc://` or `ircs://` server URL as the first argument. The nickname comes from
`IRC_NICK` and defaults to `papo-echo`.

```sh
IRC_NICK=papo-echo cargo run -p papo-echo-bot -- ircs://irc.example.net
```

Sample output after a failed action:

```text
could not answer: connection lost
```
