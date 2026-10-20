//! Nickname regain transcript tests.
mod support;

use std::time::Duration;

use papo_core::{Config, Event};
use support::Transcript;

fn registered_after_collision(config: Config, monitor: bool) -> Transcript {
    Transcript::new(config.alt_nicks(["papo_"]))
        .connect_default()
        .server(":ergo.test 433 * papo :Nickname is already in use")
        .client("NICK papo_")
        .server(":ergo.test CAP * LS :")
        .client("CAP END")
        .welcome(":ergo.test 001 papo_ :Welcome to Ergo")
        .server(if monitor {
            ":ergo.test 005 papo_ MONITOR=100 :are supported by this server"
        } else {
            ":irc.solanum.example 005 papo_ CASEMAPPING=rfc1459 :are supported by this server"
        })
        .server(":ergo.test 422 papo_ :MOTD File is missing")
        .event(|event| matches!(event, Event::Connected(_)))
}

#[test]
fn reclaims_the_primary_nick_when_ergo_reports_it_offline() {
    let mut config = Config::default();
    config.regain_nick = true;
    registered_after_collision(config, true)
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
fn an_existing_primary_monitor_is_not_added_or_removed_by_regain() {
    let mut config = Config::default();
    config.regain_nick = true;
    Transcript::new(config.alt_nicks(["papo_"]))
        .connect_default()
        .update(|session| session.monitor_add(["PAPO"]))
        .server(":ergo.test 433 * papo :Nickname is already in use")
        .client("NICK papo_")
        .server(":ergo.test CAP * LS :")
        .client("CAP END")
        .welcome(":ergo.test 001 papo_ :Welcome to Ergo")
        .server(":ergo.test 005 papo_ MONITOR=100 :are supported by this server")
        .server(":ergo.test 422 papo_ :MOTD File is missing")
        .event(|event| matches!(event, Event::Connected(_)))
        .client("MONITOR + PAPO")
        .no_client()
        .server(":ergo.test 731 papo_ :papo")
        .client("NICK papo")
        .event(|event| matches!(event, Event::MonitorOffline { nick } if nick == "papo"))
        .server(":papo_!~papo@user/papo NICK :papo")
        .event(|event| matches!(event, Event::NickChanged { new, .. } if new == "papo"))
        .no_client();
}

#[test]
fn unrelated_nick_changes_do_not_stop_monitor_regain() {
    let mut config = Config::default();
    config.regain_nick = true;
    registered_after_collision(config, true)
        .client("MONITOR + papo")
        .server(":alice!~alice@users.example NICK :alice_")
        .event(|event| matches!(event, Event::NickChanged { old, new } if old == "alice" && new == "alice_"))
        .server(":ergo.test 731 papo_ :papo")
        .client("NICK papo")
        .event(|event| matches!(event, Event::MonitorOffline { nick } if nick == "papo"))
        .ignore_rest();
}

#[test]
fn a_user_added_primary_monitor_survives_successful_regain() {
    let mut config = Config::default();
    config.regain_nick = true;
    registered_after_collision(config, true)
        .client("MONITOR + papo")
        .update(|session| session.monitor_add(["papo"]))
        .client("MONITOR + papo")
        .server(":ergo.test 731 papo_ :papo")
        .client("NICK papo")
        .event(|event| matches!(event, Event::MonitorOffline { nick } if nick == "papo"))
        .server(":papo_!~papo@user/papo NICK :papo")
        .event(|event| matches!(event, Event::NickChanged { new, .. } if new == "papo"))
        .no_client();
}

#[test]
fn monitor_regain_retries_only_after_a_matching_collision() {
    let mut config = Config::default();
    config.regain_nick = true;
    registered_after_collision(config, true)
        .client("MONITOR + papo")
        .server(":ergo.test 731 papo_ :papo")
        .client("NICK papo")
        .event(|event| matches!(event, Event::MonitorOffline { nick } if nick == "papo"))
        .server(":ergo.test 730 papo_ :papo")
        .event(|event| matches!(event, Event::MonitorOnline { nick, .. } if nick == "papo"))
        .server(":ergo.test 731 papo_ :papo")
        .event(|event| matches!(event, Event::MonitorOffline { nick } if nick == "papo"))
        .no_client()
        .server(":ergo.test 433 papo_ alice :Nickname is already in use")
        .event(|event| matches!(event, Event::Line(line) if line.as_ref().command_is("433")))
        .server(":ergo.test 731 papo_ :papo")
        .no_client();
}

#[test]
fn nickname_regain_is_disabled_by_default() {
    registered_after_collision(
        Config::default().ping_interval(Duration::from_secs(300)),
        false,
    )
    .advance(Duration::from_secs(60))
    .no_client();
}

#[test]
fn ison_replies_drive_bounded_polling_and_reclaim() {
    let mut config = Config::default()
        .ping_interval(Duration::from_secs(300))
        .ping_timeout(Duration::from_secs(600));
    config.regain_nick = true;
    registered_after_collision(config, false)
        .advance(Duration::from_secs(60))
        .client("ISON papo")
        .advance(Duration::from_secs(60))
        .no_client()
        .server(":irc.solanum.example 303 papo_")
        .event(|event| matches!(event, Event::Line(line) if line.as_ref().command_is("303")))
        .server(":irc.solanum.example 303 papo_ :papo")
        .advance(Duration::from_secs(60))
        .client("ISON papo")
        .server(":irc.solanum.example 303 papo_ :")
        .client("NICK papo")
        .server(":irc.solanum.example 433 papo_ papo :Nickname is already in use")
        .event(|event| matches!(event, Event::Line(line) if line.as_ref().command_is("433")))
        .advance(Duration::from_secs(60))
        .client("ISON papo")
        .server(":irc.solanum.example 303 papo_ :")
        .client("NICK papo")
        .server(":papo_!~papo@user/papo NICK :papo")
        .event(|event| matches!(event, Event::NickChanged { new, .. } if new == "papo"))
        .no_client();
}
