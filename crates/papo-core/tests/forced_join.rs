//! Transcript coverage for server-forced channel joins.

mod support;

use papo_core::{Authorship, Config, Event, WhoOnJoin};
use support::Transcript;

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
