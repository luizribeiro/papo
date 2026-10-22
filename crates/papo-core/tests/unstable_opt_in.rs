//! Stable-shape transcript tests for disabled unstable extensions.

mod support;

use papo_core::{Config, Event, Refusal};
#[cfg(feature = "unstable")]
use papo_core::{Feature, Session};
#[cfg(feature = "unstable")]
use papo_proto::Line;
use support::Transcript;

#[test]
fn unstable_lines_remain_untyped_without_feature_selection() {
    Transcript::new(Config::default())
        .server(":ergo.test PERSISTENCE STATUS DEFAULT ON")
        .event(
            |event| matches!(event, Event::Line(line) if line.as_ref().command_is("PERSISTENCE")),
        )
        .server("BOUNCER NETWORK libera name=Libera\\sChat;state=connected")
        .event(|event| matches!(event, Event::Line(line) if line.as_ref().command_is("BOUNCER")))
        .server("BATCH +networks soju.im/bouncer-networks")
        .server("@batch=networks BOUNCER NETWORK oftc name=OFTC;state=disconnected")
        .event(|event| matches!(event, Event::Line(line) if line.as_ref().command_is("BOUNCER")))
        .server(":ergo.test FAIL RELAYMSG BANNED #bridged :Relayed sender is banned")
        .event(|event| {
            matches!(event, Event::StandardReply(reply) if reply.refusal() == Some(Refusal::Other))
        });
}

#[test]
fn stable_events_keep_their_shape_with_optional_api_compiled() {
    Transcript::new(Config::default())
        .server(":alice!~alice@example.test JOIN #papo * :Alice Example")
        .event(|event| matches!(event, Event::Joined { channel, .. } if channel == "#papo"))
        .server(":alice!~alice@example.test PRIVMSG #papo :hello from Ergo")
        .event(|event| matches!(event, Event::Line(line) if line.as_ref().command_is("PRIVMSG")));
}

#[cfg(feature = "unstable")]
#[test]
fn enabled_extensions_type_correlated_server_errors() {
    for (feature, cap, command, code, expected) in [
        (
            Feature::Bouncer,
            "soju.im/bouncer-networks",
            "BOUNCER",
            "INVALID_NETID",
            Refusal::InvalidNetwork,
        ),
        (
            Feature::Relay,
            "draft/relaymsg",
            "RELAYMSG",
            "NOT_ENABLED",
            Refusal::RelayDisabled,
        ),
        (
            Feature::Persistence,
            "draft/persistence",
            "PERSISTENCE",
            "INVALID_PARAMS",
            Refusal::InvalidPersistenceParameters,
        ),
        (
            Feature::WebPush,
            "draft/webpush",
            "WEBPUSH",
            "MAX_REGISTRATIONS",
            Refusal::WebPushRegistrationLimit,
        ),
    ] {
        let now = std::time::Instant::now();
        let mut session = Session::new(
            Config::default()
                .enable(feature)
                .wanted_caps([cap, "labeled-response"]),
        );
        session.handle_connected(now);
        while session.poll_transmit().is_some() {}
        session.handle_line(now, format!("CAP * LS :{cap} labeled-response").as_bytes());
        let requested = String::from_utf8(session.poll_transmit().unwrap()).unwrap();
        assert!(requested.contains(cap), "{requested}");
        while session.poll_transmit().is_some() {}
        session.handle_line(now, format!("CAP * ACK :{cap} labeled-response").as_bytes());
        assert_eq!(session.supports(feature), Ok(()));
        while session.poll_transmit().is_some() {}
        session
            .request_silent(now, Line::builder(command).with_params(["probe"]))
            .unwrap();
        session.handle_line(
            now,
            format!("@label=0 :ergo.test FAIL {command} {code} :Request rejected").as_bytes(),
        );
        let Some(Event::Response(response)) = session.poll_event() else {
            panic!("expected a response for {feature:?}")
        };
        assert_eq!(response.refusal(), Some(expected));
        assert_eq!(response.error().unwrap().refusal(), expected);
    }
}
