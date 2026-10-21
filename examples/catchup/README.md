# Catchup

This example logs in, prints unread history for one channel, and advances the account's read
marker to the newest message. The channel comes from `IRC_CHANNEL` and defaults to `#papo`.

Pass an `irc://` or `ircs://` server URL as the first argument. Set `IRC_NICK` to the account
name and `IRC_PASSWORD` to its SASL PLAIN password.

```sh
IRC_NICK=alice IRC_PASSWORD='correct horse battery staple' \
  cargo run -p papo-catchup -- ircs://irc.example.net
```

Sample output:

```text
#papo deployment started
#papo deployment complete
```
