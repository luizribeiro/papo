#![no_main]

use libfuzzer_sys::fuzz_target;
use papo_core::server_url::ServerUrl;

fuzz_target!(|input: &str| {
    let Ok(parsed) = input.parse::<ServerUrl>() else {
        return;
    };
    let displayed = parsed.to_string();
    let reparsed = displayed
        .parse::<ServerUrl>()
        .expect("displayed server URLs must parse");
    assert_eq!(reparsed, parsed);
});
