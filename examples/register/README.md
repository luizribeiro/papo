# Register

This example creates an Ergo account during connection setup and prints the authenticated account.
If the server requires email verification, it prints the server's instructions and reads the
verification code from standard input before registration continues.

Configuration comes from the environment:

- `IRC_SERVER`: a host, host and port, or `irc`, `ircs`, `ws`, or `wss` URL;
- `IRC_ACCOUNT`: the new account name and initial nickname;
- `IRC_PASSWORD_FILE`: a file containing the new account password;
- `IRC_EMAIL`: an optional registration email address;
- `IRC_CA_FILE`: an optional PEM file containing the trusted CA certificate.

Run it from the repository root:

```sh
IRC_SERVER=ircs://irc.example.test:6697 IRC_ACCOUNT=alice \
IRC_PASSWORD_FILE=/run/credentials/irc-password \
IRC_EMAIL=alice@example.test IRC_CA_FILE=/run/credentials/irc-ca.pem \
cargo run -p papo-register
```

Successful output is line-oriented:

```text
logged in as alice
```

Passwords are read from a file so they do not appear in the process arguments or environment.
