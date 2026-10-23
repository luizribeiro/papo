# Bouncer networks

This example lists the networks managed by a soju bouncer, adds another upstream, and opens a
second client bound to the new network. That client joins `#papo` and prints when the channel is
ready.

Pass the bouncer URL and the new upstream's `irc://` or `ircs://` URL with an explicit port as
arguments. Set `IRC_NICK` to the upstream nickname, `IRC_ACCOUNT` to the soju account, and
`IRC_PASSWORD` to its password.

```sh
IRC_NICK=alice IRC_ACCOUNT=alice IRC_PASSWORD='correct horse battery staple' \
  cargo run -p papo-bouncer-networks -- \
  ircs://bouncer.example.net ircs://irc.oftc.net:6697
```

Sample output:

```text
network 1: Libera Chat irc.libera.chat Connected
changed 2: second irc.oftc.net Connecting
attached 2 #papo
```
