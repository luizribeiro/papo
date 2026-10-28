# Bouncer networks

This example lists the networks managed by a soju bouncer, adds another upstream, and opens a
second client bound to the new network. That client joins `#papo` and prints when the channel is
ready. The bound client's builder comes from `manager.bouncer().network(&id)`, so it reuses the
control connection's endpoint, credentials, TLS settings, and reconnect policy. Only the control
connection subscribes to network-list changes, so each change is reported once. The example
awaits `Client::quit` for both connections before exiting.

Pass the bouncer URL and the new upstream's `irc://` or `ircs://` URL as arguments. Set `IRC_NICK`
to the upstream nickname, `IRC_ACCOUNT` to the soju account, and `IRC_PASSWORD` to its password.

```sh
IRC_NICK=alice IRC_ACCOUNT=alice IRC_PASSWORD='correct horse battery staple' \
  cargo run -p papo-bouncer-networks -- \
  ircs://bouncer.example.net ircs://irc.oftc.net
```

Sample output:

```text
network 1: Libera Chat irc.libera.chat connected
changed 2: second irc.oftc.net connecting
changed 2: second irc.oftc.net connected
attached 2 #papo
```
