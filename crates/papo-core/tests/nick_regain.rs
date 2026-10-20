//! Nickname regain transcript tests.
mod support;

use papo_core::{Config, Event};
use support::Transcript;

fn registered_after_collision(config: Config) -> Transcript {
    Transcript::new(config.alt_nicks(["papo_"]))
        .connect_default()
        .server(":ergo.test 433 * papo :Nickname is already in use")
        .client("NICK papo_")
        .server(":ergo.test CAP * LS :")
        .client("CAP END")
        .welcome(":ergo.test 001 papo_ :Welcome to Ergo")
        .server(":ergo.test 005 papo_ MONITOR=100 :are supported by this server")
        .server(":ergo.test 422 papo_ :MOTD File is missing")
        .event(|event| matches!(event, Event::Connected(_)))
}

#[test]
fn reclaims_the_primary_nick_when_ergo_reports_it_offline() {
    let mut config = Config::default();
    config.regain_nick = true;
    registered_after_collision(config)
        .client("MONITOR + papo")
        .server(":ergo.test 731 papo_ :papo")
        .client("NICK papo")
        .event(|event| matches!(event, Event::MonitorOffline { nick } if nick == "papo"))
        .server(":ergo.test 731 papo_ :papo")
        .no_client()
        .server(":papo_!~papo@user/papo NICK :papo")
        .event(|event| matches!(event, Event::NickChanged { old, new } if old == "papo_" && new == "papo"))
        .client("MONITOR - papo")
        .no_client();
}

#[test]
fn nickname_regain_is_disabled_by_default() {
    registered_after_collision(Config::default()).no_client();
}
