#![no_main]

use libfuzzer_sys::fuzz_target;
use papo_proto::MessageRef;

fuzz_target!(|input: &[u8]| {
    let Ok(parsed) = MessageRef::parse_bytes(input) else {
        return;
    };
    if parsed.raw_source().is_some() {
        return;
    }

    let owned = parsed.to_owned();
    let Ok(line) = owned.to_line() else {
        return;
    };
    let reparsed = MessageRef::parse(&line).expect("serialized messages must parse");
    assert_eq!(reparsed.to_owned(), owned);
});
