# Create account

This example creates an account during connection setup. If the server requires verification,
it prints the instructions and reads the verification code from standard input.

Pass an `irc://` or `ircs://` server URL as the first argument. Set `IRC_NICK` to the new account
name, `IRC_PASSWORD` to its password, and optionally set `IRC_EMAIL`.

```sh
IRC_NICK=alice IRC_PASSWORD='correct horse battery staple' \
  cargo run -p papo-create-account -- ircs://irc.example.net
```

Sample output:

```text
logged in as alice
```
