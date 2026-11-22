#![no_main]

use libfuzzer_sys::fuzz_target;
use papo::__testing::{encode_saved_message, parse_saved_message};

fuzz_target!(|input: &str| {
    if let Some((address, ids)) = parse_saved_message(input) {
        assert_eq!(encode_saved_message(&address, &ids), input);
    }
});
