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
            papo_core::Secret::new("correct horse battery staple"),
        ),
        "draft/account-registration",
    );
}

#[test]
fn account_creation_requires_before_connect_registration() {
    Transcript::new(Config::default().create_account_on_connect(
        Some("alice"),
        Some("alice@example.org"),
        papo_core::Secret::new("correct horse battery staple"),
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
fn account_creation_acknowledged_only_after_the_welcome_sends_no_password() {
    Transcript::new(Config::default().create_account_on_connect(
        Some("alice"),
        Some("alice@example.org"),
        papo_core::Secret::new("correct horse battery staple"),
    ))
    .connect_default()
    .server(":ergo.test CAP * LS :draft/account-registration=before-connect,custom-account-name")
    .client("CAP REQ :draft/account-registration")
    .server(":ergo.test 001 papo :Welcome to the Ergo test network")
    .event(|event| {
        matches!(event, Event::Close {
            kind: CloseKind::CapabilityUnavailable,
            reason,
            ..
        } if reason.contains("draft/account-registration") && reason.contains("welcome"))
    })
    .event(|event| matches!(event, Event::Line(line) if line.as_ref().command_is("001")))
    .server(":ergo.test CAP * ACK :draft/account-registration")
    .no_client();
}

#[test]
fn custom_account_name_requires_its_capability_value() {
    Transcript::new(Config::default().create_account_on_connect(
        Some("alice"),
        Some("alice@example.org"),
        papo_core::Secret::new("correct horse battery staple"),
    ))
    .connect_default()
    .server("CAP * LS :draft/account-registration=before-connect")
    .event(|event| {
        matches!(event, Event::Close {
            kind: CloseKind::CapabilityUnavailable,
            reason,
            ..
        } if reason.contains("draft/account-registration")
            && reason.contains("custom-account-name"))
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

#[test]
fn reconnect_history_requires_chathistory() {
    let mut config = Config::default();
    config.__set_capability_required(papo_core::cap::CHATHISTORY.clone(), true);
    unavailable(config, "draft/chathistory");
}

#[test]
fn disabled_reconnect_history_is_optional() {
    let mut config = Config::default();
    config.__set_capability_required(papo_core::cap::CHATHISTORY.clone(), true);
    config.__set_capability_required(papo_core::cap::CHATHISTORY.clone(), false);
    Transcript::new(config)
        .connect_default()
        .server("CAP * LS :")
        .client("CAP END");
}
