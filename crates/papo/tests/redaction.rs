//! Public credential formatting invariants.

use papo::{Client, NewAccount, PrivateKey, Sasl, Secret, WebIrc};

fn assert_redacted(value: &impl std::fmt::Debug, secret: &str) {
    let output = format!("{value:?}");
    assert!(output.contains("[REDACTED]"));
    assert!(!output.contains(secret));
}

#[test]
fn public_credentials_are_redacted_from_debug_output() {
    let sasl_password = "unique-sasl-password-4017";
    for sasl in [
        Sasl::plain("alice", Secret::new(sasl_password)),
        Sasl::scram_sha256("alice", Secret::new(sasl_password)),
        Sasl::any("alice", Secret::new(sasl_password)),
    ] {
        assert_redacted(&sasl, sasl_password);
    }

    let account_password = "unique-account-password-6371";
    assert_redacted(
        &NewAccount::new(Secret::new(account_password)),
        account_password,
    );

    let webirc_password = "unique-webirc-password-5832";
    assert_redacted(
        &WebIrc::new(
            "papo-gateway",
            Secret::new(webirc_password),
            "192.0.2.42".parse().unwrap(),
        )
        .hostname("client.example.org")
        .flag("secure"),
        webirc_password,
    );

    let private_key = "unique-private-key-7821";
    assert_redacted(
        &PrivateKey::new(private_key.as_bytes().to_vec()),
        private_key,
    );
}

#[test]
fn runtime_handles_have_opaque_debug_output() {
    let (client, events, driver) = Client::builder("irc://irc.example.net", "papo-client")
        .password(Secret::new("unique-network-password-1597"))
        .build();
    assert_eq!(format!("{client:?}"), "Client { .. }");
    assert_eq!(format!("{events:?}"), "Events { .. }");
    assert_eq!(format!("{driver:?}"), "Driver { .. }");
    assert_eq!(
        format!("{:?}", client.quit("Leaving")),
        "RequestFuture { .. }"
    );
}

#[test]
fn request_builders_and_extension_handles_have_opaque_debug_output() {
    let autojoin_key = "unique-autojoin-key-5384";
    let builder = Client::builder("irc://irc.example.net", "papo-client")
        .autojoin_with_key("#staff", Secret::new(autojoin_key));
    assert_eq!(format!("{builder:?}"), "Builder { .. }");
    assert!(!format!("{builder:?}").contains(autojoin_key));

    let (client, _events, _driver) = builder.build();
    assert_eq!(
        format!("{:?}", client.privmsg("#papo", "Hello")),
        "SendMessage { .. }"
    );
    assert_eq!(
        format!("{:?}", client.join("#papo").key(Secret::new("secret"))),
        "JoinChannel { .. }"
    );
    assert_eq!(
        format!(
            "{:?}",
            client
                .kickban("#papo")
                .nick("mallory")
                .ban("*!*@gateway.example")
                .reason("credential leak")
        ),
        "Kickban { .. }"
    );
    assert_eq!(format!("{:?}", client.list()), "ListRequest { .. }");
    assert_eq!(format!("{:?}", client.mode("#papo")), "SetModes { .. }");
    assert_eq!(
        format!(
            "{:?}",
            client.oper("admin", Secret::new("unique-oper-password-6483"))
        ),
        "RequestFuture { .. }"
    );
    assert_eq!(
        format!(
            "{:?}",
            client.verify_account("alice", Secret::new("unique-verification-code-7319"))
        ),
        "RequestFuture { .. }"
    );
    assert_eq!(format!("{:?}", client.history("#papo")), "History { .. }");
    assert_eq!(
        format!("{:?}", client.history("#papo").latest()),
        "HistoryRequest { .. }"
    );
    assert_eq!(format!("{:?}", client.metadata()), "Metadata { .. }");
    assert_eq!(format!("{:?}", client.monitor()), "Monitor { .. }");
}

#[cfg(feature = "unstable")]
#[test]
fn unstable_extension_handles_have_opaque_debug_output() {
    let push_auth = "unique-webpush-auth-secret-4862";
    assert_redacted(
        &papo::PushKeys::new("AQIDBAUGBwgJCgsMDQ4P_w", Secret::new(push_auth)),
        push_auth,
    );
    let (client, _events, _driver) =
        Client::builder("irc://irc.example.net", "papo-client").build();
    assert_eq!(format!("{:?}", client.bouncer()), "Bouncer { .. }");
    assert_eq!(
        format!(
            "{:?}",
            client
                .bouncer()
                .add("ircs://irc.example.org")
                .password(Secret::new("unique-upstream-password-9631"))
        ),
        "AddNetwork { .. }"
    );
    assert_eq!(
        format!(
            "{:?}",
            client
                .bouncer()
                .change(&papo::NetworkId::from("libera"))
                .password(Secret::new("unique-upstream-password-1742"))
        ),
        "ChangeNetwork { .. }"
    );
    assert_eq!(format!("{:?}", client.persistence()), "Persistence { .. }");
    assert_eq!(format!("{:?}", client.webpush()), "WebPush { .. }");
}
