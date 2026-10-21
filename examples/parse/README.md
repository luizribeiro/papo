# Parse IRC lines

This example reads IRC lines from standard input and prints each line's typed command, parsed
source, unescaped tags, and parameters.

Run it from the repository root:

```sh
printf ':alice!~alice@users.example.test PRIVMSG #papo :hello\n' \
  | cargo run -p papo-parse-example --bin papo-parse
```

Sample output:

```text
command=Privmsg source=Some(User { nick: "alice", username: Some("~alice"), host: Some("users.example.test") }) tags=[] params=["#papo", "hello"]
```
