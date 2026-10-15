//! Account registration against the pinned Ergo test server.
#![cfg(any(
    not(target_os = "wasi"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

use papo_test_server::{from_env, register_account, unique};

#[test]
fn registers_an_account_with_live_ergo() {
    let server = from_env();
    let account = unique("papo-account");
    register_account(&server, &account, "q7Hx-2mZp_9vLcT4wRfB").unwrap();
    assert!(register_account(&server, &account, "q7Hx-2mZp_9vLcT4wRfB").is_err());
}
