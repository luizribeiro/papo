//! Transcript coverage for server-forced channel joins.

mod support;

use std::time::Instant;

use papo_core::{Authorship, Config, Event, RateLimit, Refusal, Session, WhoOnJoin};
use support::Transcript;

fn server(session: &mut Session, now: Instant, line: &str) {
    session.handle_line(now, line.as_bytes());
}

fn request_session() -> Session {
    Session::new(
        Config::default()
            .nick("papo")
            .rate_limit(RateLimit::unlimited())
            .who_on_join(WhoOnJoin::Never),
    )
}

#[test]
fn unrequested_own_join_builds_a_normal_membership() {
    let mut guard = None;
    Transcript::new(
        Config::default()
            .wanted_caps(["no-implicit-names"])
            .who_on_join(WhoOnJoin::Never),
    )
    .connect_default()
    .server("CAP * LS :no-implicit-names")
    .client("CAP REQ :no-implicit-names")
    .server("CAP * ACK :no-implicit-names")
    .client("CAP END")
    .server(":ergo.test 005 papo PREFIX=(ov)@+ :are supported")
    .server(":papo!~papo@localhost JOIN #release * :Papo Bot")
    .client("NAMES #release")
    .event(|event| {
        matches!(
            event,
            Event::Joined {
                channel,
                nick,
                authorship: Authorship::Own,
                ..
            } if channel == "#release" && nick == "papo"
        )
    })
    .inspect(|session| {
        let state = session.state();
        let channel = state.channel("#release").unwrap();
        assert!(!channel.is_synced());
        assert!(channel.member("papo").is_some());
        assert_eq!(state.user("papo").unwrap().channels().count(), 1);
        guard = Some(state.__channel_guard(channel.name()));
    })
    .server(":ergo.test 353 papo = #release :@papo alice")
    .server(":ergo.test 366 papo #release :End of /NAMES list")
    .client("MODE #release")
    .event(|event| matches!(event, Event::ChannelSynced { channel, .. } if channel == "#release"))
    .inspect(|session| {
        let state = session.state();
        assert!(state.channel("#release").unwrap().is_synced());
        assert_eq!(
            state
                .__resolve_channel(guard.as_ref().unwrap())
                .as_ref()
                .map(papo_proto::ChannelName::as_str),
            Some("#release")
        );
    })
    .server(":papo!~papo@localhost PART #release :Deployment complete")
    .event(|event| matches!(event, Event::Parted { channel, .. } if channel == "#release"))
    .inspect(|session| {
        assert!(session.state().channel("#release").is_none());
        assert_eq!(
            session.state().__resolve_channel(guard.as_ref().unwrap()),
            None
        );
    })
    .no_event();
}

#[test]
fn unrelated_forced_join_leaves_the_requested_join_pending() {
    let now = Instant::now();
    let mut session = request_session();
    let id = session.join(now, "#review", None).unwrap();
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"JOIN #review\r\n"
    );

    server(
        &mut session,
        now,
        ":papo!~papo@localhost JOIN #operations * :Papo Bot",
    );
    assert!(matches!(
        session.poll_event(),
        Some(Event::Joined { channel, .. }) if channel == "#operations"
    ));
    assert_eq!(session.poll_event(), None);
    server(
        &mut session,
        now,
        ":ergo.test 366 papo #operations :End of /NAMES list",
    );
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"MODE #operations\r\n"
    );
    assert!(
        std::iter::from_fn(|| session.poll_event())
            .all(|event| !matches!(event, Event::Response(response) if response.id == id))
    );

    server(
        &mut session,
        now,
        ":papo!~papo@localhost JOIN #review * :Papo Bot",
    );
    server(
        &mut session,
        now,
        ":ergo.test 366 papo #review :End of /NAMES list",
    );
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"MODE #review\r\n"
    );
    let events = std::iter::from_fn(|| session.poll_event()).collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::Response(response) if response.id == id))
            .count(),
        1
    );
    assert!(session.state().channel("#operations").is_some());
    assert!(session.state().channel("#review").is_some());
}

#[test]
fn solanum_forward_resolves_the_original_join_with_470() {
    let now = Instant::now();
    let mut session = request_session();
    let id = session.join(now, "#staff", None).unwrap();
    assert!(session.poll_transmit().is_some());

    server(
        &mut session,
        now,
        ":irc.papo.test 470 papo #staff #operations :Forwarding to another channel",
    );
    let Some(Event::Response(response)) = session.poll_event() else {
        panic!("forward numeric did not resolve the original JOIN")
    };
    assert_eq!(response.id, id);
    assert_eq!(response.error().unwrap().refusal(), Refusal::Other);
    assert_eq!(response.lines[0].as_ref().raw_command(), "470");

    server(
        &mut session,
        now,
        ":papo!~papo@localhost JOIN #operations * :Papo Bot",
    );
    server(
        &mut session,
        now,
        ":irc.papo.test 366 papo #operations :End of /NAMES list.",
    );
    assert_eq!(
        session.poll_transmit().unwrap().as_ref(),
        b"MODE #operations\r\n"
    );
    let events = std::iter::from_fn(|| session.poll_event()).collect::<Vec<_>>();
    assert!(
        events.iter().any(
            |event| matches!(event, Event::Joined { channel, .. } if channel == "#operations")
        )
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::Response(response) if response.id == id))
    );
    assert!(session.state().channel("#staff").is_none());
    assert!(session.state().channel("#operations").is_some());
}
