//! Account registration against the pinned Ergo test server.
#![cfg(any(
    not(target_os = "wasi"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

use papo_test_server::{change_account_password_at, create_account, from_env, unique};

#[test]
fn registers_an_account_with_live_ergo() {
    let server = from_env();
    let account = unique("papo-account");
    create_account(&server, &account, "q7Hx-2mZp_9vLcT4wRfB").unwrap();
    assert!(create_account(&server, &account, "q7Hx-2mZp_9vLcT4wRfB").is_err());
}

#[test]
fn changes_an_account_password_mid_session() {
    let server = from_env();
    let account = unique("papo-password");
    let first = "q7Hx-2mZp_9vLcT4wRfB";
    let second = "r8Ky-3nAq_6wMdP5xTgC";
    let third = "s9Lz-4pBr_7xNeQ6yUhD";
    create_account(&server, &account, first).unwrap();
    let address = server.bouncer().map_or_else(
        || server.plaintext_addr(),
        papo_test_server::BouncerInfo::upstream_addr,
    );
    change_account_password_at(address, &account, first, second).unwrap();
    change_account_password_at(address, &account, second, third).unwrap();
}
