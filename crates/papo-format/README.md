# papo-format

Build, parse, and strip IRC text formatting. This crate has no stability promise.

## Send bold and colored text

Build the text and pass the result to `client.privmsg("#deployments", message)`.

```rust
use papo_format::FormatBuilder;

let mut text = FormatBuilder::new();
text.bold()
    .text("Deployment complete")
    .reset()
    .text(" on ")
    .color(12, None)
    .text("Ergo")
    .reset();
let message = text.finish();
assert_eq!(papo_format::strip_formatting(&message), "Deployment complete on Ergo");
```

## Read styled runs

Pass a received message or notice's `raw_text` to `Formatted::new`.

```rust
use papo_format::Formatted;

let raw_text = "deployment \x02complete\x02 on Ergo";
let runs: Vec<_> = Formatted::new(raw_text).collect();
assert_eq!(runs[1].text, "complete");
assert!(runs[1].style.bold);
```
