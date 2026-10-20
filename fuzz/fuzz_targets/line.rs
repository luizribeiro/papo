#![no_main]

use libfuzzer_sys::fuzz_target;
use papo_proto::LineRef;

fuzz_target!(|input: &[u8]| {
    let Ok(parsed) = LineRef::parse_bytes(input) else {
        return;
    };
    if parsed.raw_source().is_some() {
        return;
    }

    let owned = parsed.to_owned();
    let Ok(line) = owned.to_line() else {
        return;
    };
    let reparsed = LineRef::parse(&line).expect("serialized lines must parse");
    assert_eq!(reparsed.to_owned(), owned);
});
