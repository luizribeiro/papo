#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    papo::__testing::exercise_session_mapping(input, |_| {});
});
