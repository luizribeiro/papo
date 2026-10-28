# Catchup

This example logs in, prints up to 100 unread messages for one channel, and advances the account's
read marker to the last message it printed. It then awaits `Client::quit` before exiting. The
channel comes from `IRC_CHANNEL` and defaults to `#papo`; `IRC_NICK` defaults to `papo-catchup`.
If history or read-marker handling fails, it prints `could not catch up: …` and still quits.

Pass an `irc://` or `ircs://` server URL as the first argument. Set `IRC_NICK` to the account
name and `IRC_PASSWORD` to its SASL PLAIN password. Plaintext `irc://` with credentials also needs
`IRC_ALLOW_INSECURE_AUTH=1`; use `ircs://` normally.

```sh
IRC_NICK=alice IRC_PASSWORD='correct horse battery staple' \
  cargo run -p papo-catchup -- ircs://irc.example.net
```

Sample output:

```text
#papo the deployment started
#papo the migration completed
#papo the service is healthy
```
