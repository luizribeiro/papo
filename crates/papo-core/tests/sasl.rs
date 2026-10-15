//! Transcript tests for SASL authentication.

use papo_core::{Config, Entropy, Sasl, Session};

struct FixedEntropy;

impl Entropy for FixedEntropy {
    fn fill(&mut self, buf: &mut [u8]) {
        buf.fill(0x5a);
    }
}

#[test]
fn configures_mechanisms_policy_and_entropy() {
    let config = Config::default()
        .sasl(Sasl::any("alice", "correct horse battery staple"))
        .require_sasl(true)
        .device("phone");
    assert!(config.sasl.is_some());
    assert!(config.require_sasl);
    assert_eq!(config.device.as_deref(), Some("phone"));

    let session = Session::with_entropy(config, Box::new(FixedEntropy));
    assert!(format!("{session:?}").contains("has_entropy: true"));
}

#[test]
fn external_accepts_an_authorization_identity() {
    assert_ne!(Sasl::external(), Sasl::external().authzid("alice"));
}
