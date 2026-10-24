//! Transcript tests for capabilities implied by connection settings.

mod support;

use papo_core::{CloseKind, Config, Event};
use support::Transcript;

fn unavailable(config: Config, capability: &str) {
    Transcript::new(config)
        .connect_default()
        .server("CAP * LS :batch")
        .event(|event| {
            matches!(event, Event::Close {
                kind: CloseKind::CapabilityUnavailable,
                reason,
                ..
            } if reason.contains(capability))
        })
        .no_client();
}

#[test]
fn account_creation_requires_account_registration() {
    unavailable(
        Config::default().create_account_on_connect(
            Some("alice"),
            Some("alice@example.org"),
            "correct horse battery staple",
        ),
        "draft/account-registration",
    );
}

#[test]
fn account_creation_requires_before_connect_registration() {
    Transcript::new(Config::default().create_account_on_connect(
        Some("alice"),
        Some("alice@example.org"),
        "correct horse battery staple",
    ))
    .connect_default()
    .server("CAP * LS :draft/account-registration=email-required")
    .event(|event| {
        matches!(event, Event::Close {
            kind: CloseKind::CapabilityUnavailable,
            reason,
            ..
        } if reason.contains("draft/account-registration") && reason.contains("before-connect"))
    })
    .no_client();
}

#[test]
fn metadata_subscriptions_require_metadata() {
    unavailable(
        Config::default().metadata_subscriptions(["example.org/color"]),
        "draft/metadata-3",
    );
}
