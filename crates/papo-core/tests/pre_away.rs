//! Transcript tests for `draft/pre-away`.

mod support;

use papo_core::{AwayOnConnect, Config, Event};
use support::Transcript;

fn config(away: AwayOnConnect, cap: bool) -> Config {
    let config = Config::default().away_on_connect(Some(away));
    if cap {
        config.wanted_caps(["draft/pre-away"])
    } else {
        config.wanted_caps(std::iter::empty::<&str>())
    }
}

#[test]
fn sends_message_and_hidden_away_before_cap_end() {
    for (away, line) in [
        (
            AwayOnConnect::Message("Reviewing patches".into()),
            "AWAY :Reviewing patches",
        ),
        (AwayOnConnect::Hidden, "AWAY *"),
    ] {
        Transcript::new(config(away, true))
            .connect_default()
            .server("CAP * LS :draft/pre-away")
            .client("CAP REQ :draft/pre-away")
            .server("CAP * ACK :draft/pre-away")
            .client(line)
            .client("CAP END");
    }
}

#[test]
fn sends_only_message_away_after_registration_without_the_capability() {
    Transcript::new(config(
        AwayOnConnect::Message("Reviewing patches".into()),
        false,
    ))
    .connect_default()
    .server("CAP * LS :")
    .client("CAP END")
    .server(":ergo.test 001 alice :Welcome to Ergo")
    .server(":ergo.test 376 alice :End of MOTD")
    .client("AWAY :Reviewing patches")
    .event(|event| matches!(event, Event::Registered(_)));

    Transcript::new(config(AwayOnConnect::Hidden, false))
        .connect_default()
        .server("CAP * LS :")
        .client("CAP END")
        .server(":ergo.test 001 alice :Welcome to Ergo")
        .server(":ergo.test 376 alice :End of MOTD")
        .event(|event| matches!(event, Event::Registered(_)))
        .no_client();
}
