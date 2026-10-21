# Reaction bot

This bot reacts with 👍 to every live message it receives. Failed reactions are logged without
stopping the bot.

Pass an `irc://` or `ircs://` server URL as the first argument. The nickname comes from
`IRC_NICK` and defaults to `papo-react`.

```sh
IRC_NICK=papo-react cargo run -p papo-react-bot -- ircs://irc.example.net
```

Sample output after a failed reaction:

```text
could not react: connection lost
```
