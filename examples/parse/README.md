# Parse IRC messages

This example reads one IRC message per line from standard input and prints its typed command,
parsed source, unescaped tags, and parameters. `PRIVMSG` and `NOTICE` output identifies CTCP
messages; other text is printed with IRC formatting controls removed.

Run it from the repository root:

```sh
printf ':alice!~alice@users.example.test PRIVMSG #papo :hello\n' \
  | cargo run -p papo-parse-example --bin papo-parse
```
