//! Stable-shape transcript tests for disabled unstable extensions.

mod support;

use papo_core::{Config, Event};
use support::Transcript;

#[test]
fn unstable_lines_remain_untyped_without_feature_selection() {
    Transcript::new(Config::default())
        .server(":ergo.test PERSISTENCE STATUS DEFAULT ON")
        .event(
            |event| matches!(event, Event::Line(line) if line.as_ref().command_is("PERSISTENCE")),
        )
        .server("BOUNCER NETWORK libera name=Libera\\sChat;state=connected")
        .event(|event| matches!(event, Event::Line(line) if line.as_ref().command_is("BOUNCER")));
}

#[test]
fn stable_events_keep_their_shape_with_optional_api_compiled() {
    Transcript::new(Config::default())
        .server(":alice!~alice@example.test JOIN #papo * :Alice Example")
        .event(|event| matches!(event, Event::Joined { channel, .. } if channel == "#papo"))
        .server(":alice!~alice@example.test PRIVMSG #papo :hello from Ergo")
        .event(|event| matches!(event, Event::Line(line) if line.as_ref().command_is("PRIVMSG")));
}
