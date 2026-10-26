//! Public credential formatting invariants.

use papo::{Client, NewAccount, Sasl, Secret, WebIrc};

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
            password: Secret::from(webirc_password),
            gateway: "papo-gateway".into(),
            client_ip: "192.0.2.42".parse().unwrap(),
            client_hostname: Some("client.example.org".into()),
            options: vec![("secure".into(), None)],
        },
        webirc_password,
    );
}

#[test]
fn runtime_handles_have_opaque_debug_output() {
    let (client, events, driver) = Client::builder("irc://irc.example.net", "papo-client")
        .password("unique-network-password-1597")
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
    let builder = Client::builder("irc://irc.example.net", "papo-client");
    assert_eq!(format!("{builder:?}"), "Builder { .. }");

    let (client, _events, _driver) = builder.build();
    assert_eq!(
        format!("{:?}", client.privmsg("#papo", "Hello")),
        "SendMessage { .. }"
    );
    assert_eq!(
        format!("{:?}", client.join("#papo").key("secret")),
        "JoinChannel { .. }"
    );
    assert_eq!(format!("{:?}", client.list_request()), "ListRequest { .. }");
    assert_eq!(format!("{:?}", client.mode("#papo")), "SetModes { .. }");
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
    let (client, _events, _driver) =
        Client::builder("irc://irc.example.net", "papo-client").build();
    assert_eq!(format!("{:?}", client.bouncer()), "Bouncer { .. }");
    assert_eq!(
        format!(
            "{:?}",
            client
                .bouncer()
                .add("ircs://irc.example.org")
                .password("unique-upstream-password-9631")
        ),
        "AddNetwork { .. }"
    );
    assert_eq!(
        format!(
            "{:?}",
            client
                .bouncer()
                .change(&papo::NetworkId::from("libera"))
                .password("unique-upstream-password-1742")
        ),
        "ChangeNetwork { .. }"
    );
    assert_eq!(format!("{:?}", client.persistence()), "Persistence { .. }");
    assert_eq!(format!("{:?}", client.webpush()), "WebPush { .. }");
}
