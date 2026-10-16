//! Public credential formatting invariants.

use papo::{NewAccount, Sasl, Secret, WebIrc};

fn assert_redacted(value: &impl std::fmt::Debug, secret: &str) {
    let output = format!("{value:?}");
    assert!(output.contains("[REDACTED]"));
    assert!(!output.contains(secret));
}

#[test]
fn public_credentials_are_redacted_from_debug_output() {
    let sasl_password = "unique-sasl-password-4017";
    assert_redacted(&Sasl::any("alice", sasl_password), sasl_password);

    let account_password = "unique-account-password-6371";
    assert_redacted(&NewAccount::new(account_password), account_password);

    let webirc_password = "unique-webirc-password-5832";
    assert_redacted(
        &WebIrc {
            password: Secret::new(webirc_password),
            gateway: "papo-gateway".into(),
            hostname: Some("client.example.org".into()),
            ip: "192.0.2.42".into(),
            options: vec![("secure".into(), None)],
        },
        webirc_password,
    );
}
