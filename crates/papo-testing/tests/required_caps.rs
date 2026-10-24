//! Connection-setting regressions for required capabilities.

use papo::{ConnectErrorKind, DisconnectCause, NewAccount, RateLimit, Reconnect, Sasl};
use papo_testing::wire;

const PASSWORD: &str = "correct horse battery staple";

async fn assert_unavailable(
    builder: papo::Builder,
    expected_kind: ConnectErrorKind,
    capability: &str,
) {
    let (_client, mut events, mut wire) = wire(
        builder
            .rate_limit(RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    assert_eq!(wire.next_sent().await, "CAP LS 302");
    assert_eq!(wire.next_sent().await, "NICK papo-wire");
    assert_eq!(wire.next_sent().await, "USER papo-wire 0 * papo-wire");
    wire.server(":offline.test CAP * LS :").await;

    loop {
        match events.next().await {
            Some(papo::Event::Disconnected(disconnected)) => {
                let DisconnectCause::Connect(error) = &disconnected.cause else {
                    panic!("unexpected disconnect: {disconnected:?}");
                };
                assert_eq!(error.kind(), expected_kind);
                assert!(
                    error.to_string().to_ascii_lowercase().contains(capability),
                    "{error}"
                );
                let diagnostic = std::error::Error::source(&error)
                    .map_or_else(|| error.to_string(), ToString::to_string);
                assert!(diagnostic.contains(capability), "{diagnostic}");
                assert!(disconnected.reconnect_in.is_none());
                break;
            }
            Some(papo::Event::Connected(_)) => panic!("required capability was ignored"),
            Some(_) => {}
            None => panic!("driver stopped without reporting the capability error"),
        }
    }
    assert_eq!(wire.next_sent_or_closed().await, None);
}

fn builder() -> papo::Builder {
    papo::Client::builder("ircs://offline.test").nick("papo-wire")
}

#[tokio::test]
async fn sasl_requires_sasl() {
    assert_unavailable(
        builder().sasl(Sasl::plain("alice", PASSWORD)),
        ConnectErrorKind::SaslUnavailable,
        "sasl",
    )
    .await;
}

#[tokio::test]
async fn account_creation_requires_account_registration() {
    assert_unavailable(
        builder().create_account(NewAccount::new(PASSWORD)),
        ConnectErrorKind::CapabilityUnavailable,
        "draft/account-registration",
    )
    .await;
}

#[tokio::test]
async fn metadata_subscriptions_require_metadata() {
    assert_unavailable(
        builder().metadata_subscriptions(["example.org/color"]),
        ConnectErrorKind::CapabilityUnavailable,
        "draft/metadata-3",
    )
    .await;
}

#[tokio::test]
async fn reconnect_history_requires_chathistory() {
    assert_unavailable(
        builder().history_on_reconnect(true),
        ConnectErrorKind::CapabilityUnavailable,
        "draft/chathistory",
    )
    .await;
}
