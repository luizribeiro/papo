#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    assert_eq!(
        papo::__testing::exercise_session_mapping_invariants(input),
        input.len()
    );
    papo::__testing::exercise_session_mapping(input, |_| {});
});
