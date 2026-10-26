use super::*;

async fn read_builder_line(server: &mut DuplexStream) -> Option<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let byte = server.read_u8().await.ok()?;
        line.push(byte);
        if byte == b'\n' {
            return Some(line);
        }
    }
}

async fn check_builder(configure: impl FnOnce(crate::Builder) -> crate::Builder) {
    let (stream, mut server) = tokio::io::duplex(4096);
    let (client, events, driver) = configure(crate::Client::builder(
        crate::Endpoint::plaintext("127.0.0.1", 6667).unwrap(),
        "papo-proptest",
    ))
    .stream(crate::driver::test_support::TokioIo(stream))
    .build();
    tokio::pin!(driver);
    let result = tokio::select! {
        result = &mut driver => Some(result),
        line = read_builder_line(&mut server) => {
            if let Some(line) = line {
                validate_line(&line);
                None
            } else {
                Some(driver.await)
            }
        }
    };
    if let Some(result) = result {
        assert!(
            matches!(
                &result,
                Err(Error::InvalidInput(_)
                    | Error::Disconnected(crate::Disconnected {
                        cause: crate::DisconnectCause::PingTimeout,
                        ..
                    }))
            ),
            "unexpected driver result: {result:?}"
        );
    }
    drop((client, events));
}

#[tokio::test(flavor = "current_thread")]
async fn arbitrary_builder_setter_input_is_rejected_or_registers_validly() {
    for (first, second, third) in inputs() {
        macro_rules! probe {
            ($builder:ident, $a:ident, $b:ident, $c:ident => $configured:expr) => {{
                let ($a, $b, $c) = (first.clone(), second.clone(), third.clone());
                check_builder(move |$builder| $configured).await;
            }};
        }
        probe!(builder, a, _b, _c => {
            drop(builder);
            crate::Client::builder(
                crate::Endpoint::plaintext("127.0.0.1", 6667).unwrap(),
                crate::Casemapping::default().nick(a),
            )
        });
        probe!(builder, a, b, _c => builder.alt_nicks([
            crate::Casemapping::default().nick(a),
            crate::Casemapping::default().nick(b),
        ]));
        probe!(builder, a, _b, _c => builder.username(a));
        probe!(builder, a, _b, _c => builder.realname(a));
        probe!(builder, a, _b, _c => builder.password(a));
        probe!(builder, a, b, c => builder.webirc(
            crate::WebIrc {
                password: c.clone().into(),
                gateway: a.clone(),
                client_ip: "192.0.2.42".parse().unwrap(),
                client_hostname: Some(b.clone()),
                options: vec![(b, Some(c.clone())), (c, None)],
            }
        ));
        probe!(builder, a, b, c => builder.sasl(crate::Sasl::plain(a, b).device(c)));
        #[cfg(feature = "unstable")]
        probe!(builder, a, _b, _c => builder.bouncer_network(crate::NetworkId::from(a)));
        probe!(builder, a, b, c => builder.create_account(
            crate::NewAccount::new(c)
                .account(crate::Casemapping::default().account(a))
                .email(b)
        ));
        probe!(builder, a, b, _c => builder.request_caps([a, b]));
        probe!(builder, _a, _b, _c => builder.disable(crate::Feature::MessageTags));
        probe!(builder, a, b, _c => builder.metadata_subscriptions([a, b]));
        probe!(builder, _a, _b, _c => builder.rate_limit(crate::RateLimit::unlimited()));
        probe!(builder, a, _b, _c => builder.ping_interval(Duration::from_secs(a.len() as u64)));
        probe!(builder, a, _b, _c => builder.ping_timeout(Duration::from_secs(a.len() as u64)));
        probe!(builder, a, _b, _c => builder.request_timeout(Duration::from_secs(a.len() as u64)));
        probe!(builder, a, _b, _c => builder.away_on_connect(a));
        probe!(builder, a, _b, _c => builder.ctcp_replies(
            crate::CtcpReplies::default().version(&a)
        ));
        probe!(builder, a, _b, _c => builder.event_capacity(a.len()));
        probe!(builder, _a, _b, _c => builder.reconnect(crate::Reconnect::never()));
        probe!(builder, a, _b, _c => builder.connect_timeout(Duration::from_secs(a.len() as u64)));
        probe!(builder, _a, _b, _c => builder.sts_store(papo_core::sts::MemoryStsStore::new()));
        probe!(builder, a, _b, _c => builder.tls_ca_pem(a.into_bytes()));
        probe!(builder, a, _b, _c => builder.tls_extra_ca_pem(a.into_bytes()));
        probe!(builder, a, b, _c => builder.tls_client_cert(a.into_bytes(), b));
        probe!(builder, a, _b, _c => builder.tls_server_name(a));
        probe!(builder, a, _b, _c => builder.bot(a.len() % 2 == 0));
        probe!(builder, a, b, _c => builder.autojoin([a, b]));
        probe!(builder, a, b, _c => builder.autojoin_with_key(a, b));
        probe!(builder, a, _b, _c => builder.history_on_reconnect(a.len() % 2 == 0));
    }
}
