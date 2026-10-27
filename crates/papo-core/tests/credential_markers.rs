//! Coverage for secret provenance on every credential-bearing command.

use std::time::Instant;

use papo_core::Feature;
use papo_core::{Config, RateLimit, Sasl, Secret, Session, WebIrc};
use papo_proto::WireLine;

fn secure(config: Config) -> Session {
    Session::new(
        config
            .connection_security(papo_core::sts::ConnectionSecurity::Secure)
            .rate_limit(RateLimit::unlimited()),
    )
}

fn drain(session: &mut Session) -> Vec<WireLine> {
    std::iter::from_fn(|| session.poll_transmit()).collect()
}

fn registered(config: Config, caps: &str) -> Session {
    let now = Instant::now();
    let mut session = secure(config);
    session.handle_connected(now);
    drain(&mut session);
    session.handle_line(now, format!(":ergo.test CAP papo LS :{caps}").as_bytes());
    session.handle_line(now, format!(":ergo.test CAP papo ACK :{caps}").as_bytes());
    drain(&mut session);
    session.handle_line(now, b":ergo.test 001 papo :Welcome to Ergo");
    session.handle_line(now, b":ergo.test 422 papo :MOTD File is missing");
    drain(&mut session);
    session
}

fn marked(command: &'static str, line: WireLine) -> (&'static str, WireLine) {
    (command, line)
}

#[test]
fn every_credential_command_marks_its_secret_parameter() {
    let now = Instant::now();
    let mut lines = Vec::new();

    let mut pass = secure(Config::default().password("server-password"));
    pass.handle_connected(now);
    drain(&mut pass);
    pass.handle_line(now, b":ergo.test CAP papo LS :");
    lines.push(marked("PASS", pass.poll_transmit().unwrap()));

    let webirc = WebIrc {
        password: Secret::new("gateway-password"),
        gateway: "papo-gateway".into(),
        client_ip: "192.0.2.42".parse().unwrap(),
        client_hostname: None,
        options: Vec::new(),
    };
    let mut gateway = secure(Config::default().webirc(webirc));
    gateway.handle_connected(now);
    lines.push(marked("WEBIRC", gateway.poll_transmit().unwrap()));

    let mut oper = secure(Config::default());
    oper.oper(now, "admin", &Secret::new("operator-password"))
        .unwrap();
    lines.push(marked("OPER", oper.poll_transmit().unwrap()));

    for (command, verify) in [("REGISTER", false), ("VERIFY", true)] {
        let cap = "draft/account-registration";
        let mut account = registered(Config::default().wanted_caps([cap]), cap);
        if verify {
            account.verify(now, "alice", "verification-code").unwrap();
        } else {
            account
                .create_account(now, None, None, "account-password")
                .unwrap();
        }
        lines.push(marked(command, account.poll_transmit().unwrap()));
    }

    let mut sasl = secure(Config::default().sasl(Sasl::plain("alice", "sasl-password")));
    sasl.handle_connected(now);
    drain(&mut sasl);
    sasl.handle_line(now, b":ergo.test CAP papo LS :sasl=PLAIN");
    sasl.handle_line(now, b":ergo.test CAP papo ACK :sasl");
    drain(&mut sasl);
    sasl.handle_line(now, b"AUTHENTICATE +");
    lines.push(marked("AUTHENTICATE", sasl.poll_transmit().unwrap()));

    let caps = "labeled-response soju.im/bouncer-networks";
    let mut bouncer = registered(
        Config::default()
            .enable(Feature::Bouncer)
            .wanted_caps(caps.split_whitespace()),
        caps,
    );
    for (command, change) in [
        ("BOUNCER ADDNETWORK", false),
        ("BOUNCER CHANGENETWORK", true),
    ] {
        let attributes = papo_core::__bouncer_attributes(
            Some("irc.libera.chat"),
            None,
            Some(6697),
            Some(true),
            None,
            None,
            None,
            Some(&Secret::new("upstream-password")),
        )
        .unwrap();
        if change {
            bouncer
                .bouncer_change(now, &papo_core::NetworkId::from("43"), attributes)
                .unwrap();
        } else {
            bouncer.bouncer_add(now, attributes).unwrap();
        }
        lines.push(marked(command, bouncer.poll_transmit().unwrap()));
    }

    for (command, line) in lines {
        assert!(line.__has_secret_parameter(), "{command} lost its marker");
    }
}
