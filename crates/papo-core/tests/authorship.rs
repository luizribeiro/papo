//! Transcript coverage for actor and subject identity facts.

mod support;

use papo_core::{Authorship, Config, Event, HistoryItem, Sasl, Subject};
use support::Transcript;

#[test]
fn live_subject_facts_are_exact() {
    Transcript::new(Config::default())
        .server(":emily!~emily@localhost KICK #papo papo :Requested")
        .event(|event| {
            matches!(
                event,
                Event::Kicked {
                    authorship: Authorship::Other,
                    subject_authorship: Authorship::Own,
                    ..
                }
            )
        })
        .server(":emily!~emily@localhost INVITE papo :#papo")
        .event(|event| {
            matches!(
                event,
                Event::Invited {
                    authorship: Authorship::Other,
                    subject_authorship: Authorship::Own,
                    ..
                }
            )
        })
        .server(":ergo.test MODE papo +i")
        .event(|event| {
            matches!(
                event,
                Event::ModeChanged {
                    subject: Subject::User(nick),
                    authorship: Authorship::Other,
                    subject_authorship: Authorship::Own,
                    ..
                } if nick == "papo"
            )
        });
}

#[test]
fn queued_event_keeps_authorship_across_a_nick_change() {
    Transcript::new(Config::default())
        .server(":papo!papo@localhost JOIN #papo")
        .server(":papo!papo@localhost NICK papo-away")
        .event(|event| {
            matches!(
                event,
                Event::Joined {
                    nick,
                    authorship: Authorship::Own,
                    ..
                } if nick == "papo"
            )
        })
        .event(|event| {
            matches!(
                event,
                Event::NickChanged {
                    old,
                    new,
                    authorship: Authorship::Own,
                    ..
                } if old == "papo" && new == "papo-away"
            )
        });
}

#[test]
fn live_message_from_another_session_of_our_account_is_own() {
    Transcript::new(Config::default().sasl(Sasl::plain("papo", "correct horse battery staple")))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN account-tag")
        .client("CAP REQ :account-tag sasl")
        .server("CAP * ACK :account-tag sasl")
        .client("AUTHENTICATE PLAIN")
        .server("AUTHENTICATE +")
        .client("AUTHENTICATE AHBhcG8AY29ycmVjdCBob3JzZSBiYXR0ZXJ5IHN0YXBsZQ==")
        .server(":ergo.test 900 papo papo!~papo@localhost papo :You are now logged in as papo")
        .server(":ergo.test 903 papo :SASL authentication successful")
        .client("CAP END")
        .server("@account=papo;msgid=RfcBTzyxY120004 :papo-phone!~papo@localhost PRIVMSG #papo :deployment finished")
        .event(|event| {
            matches!(event, Event::Line(message)
                if message.authorship() == Authorship::Own
                    && message.as_ref().param(1) == Some("deployment finished"))
        });
}

#[test]
fn sibling_device_away_is_own() {
    Transcript::new(Config::default().sasl(Sasl::plain("papo", "correct horse battery staple")))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN account-tag away-notify")
        .client("CAP REQ :account-tag away-notify sasl")
        .server("CAP * ACK :account-tag away-notify sasl")
        .client("AUTHENTICATE PLAIN")
        .server("AUTHENTICATE +")
        .client("AUTHENTICATE AHBhcG8AY29ycmVjdCBob3JzZSBiYXR0ZXJ5IHN0YXBsZQ==")
        .server(":ergo.test 900 papo papo!~papo@localhost papo :You are now logged in as papo")
        .server(":ergo.test 903 papo :SASL authentication successful")
        .client("CAP END")
        .server(":papo!~papo@localhost JOIN #papo")
        .server("@account=papo :papo-phone!~papo@localhost JOIN #papo")
        .server("@account=papo :papo-phone!~papo@localhost AWAY :reviewing the production rollout")
        .event_until(|event| {
            matches!(event, Event::AwayChanged {
                nick,
                authorship: Authorship::Own,
                ..
            } if nick == "papo-phone")
        });
}

#[test]
fn nick_change_channels_distinguish_sibling_devices_from_other_users() {
    Transcript::new(Config::default().sasl(Sasl::plain("papo", "correct horse battery staple")))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN account-tag")
        .client("CAP REQ :account-tag sasl")
        .server("CAP * ACK :account-tag sasl")
        .client("AUTHENTICATE PLAIN")
        .server("AUTHENTICATE +")
        .client("AUTHENTICATE AHBhcG8AY29ycmVjdCBob3JzZSBiYXR0ZXJ5IHN0YXBsZQ==")
        .server(":ergo.test 900 papo papo!~papo@localhost papo :You are now logged in as papo")
        .server(":ergo.test 903 papo :SASL authentication successful")
        .client("CAP END")
        .server(":papo!~papo@localhost JOIN #papo")
        .server(":papo!~papo@localhost JOIN #rust")
        .server("@account=papo :papo-phone!~papo@localhost JOIN #papo")
        .server(":emily!~emily@localhost JOIN #rust")
        .server("@account=papo :papo-phone!~papo@localhost NICK papo-mobile")
        .event_until(|event| {
            matches!(event, Event::NickChanged {
                old,
                channels,
                authorship: Authorship::Own,
                ..
            } if old == "papo-phone" && *channels == ["#papo", "#rust"])
        })
        .server(":emily!~emily@localhost NICK emily-away")
        .event(|event| {
            matches!(event, Event::NickChanged {
                old,
                channels,
                authorship: Authorship::Other,
                ..
            } if old == "emily" && *channels == ["#rust"])
        });
}

#[test]
fn history_authorship_requires_an_account_tag() {
    Transcript::new(Config::default().sasl(Sasl::plain("papo", "correct horse battery staple")))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .server("AUTHENTICATE +")
        .client("AUTHENTICATE AHBhcG8AY29ycmVjdCBob3JzZSBiYXR0ZXJ5IHN0YXBsZQ==")
        .server(":ergo.test 900 papo papo!~papo@localhost papo :You are now logged in as papo")
        .server(":ergo.test 903 papo :SASL authentication successful")
        .client("CAP END")
        .server(":ergo.test BATCH +history chathistory #papo")
        .server("@batch=history;account=papo;time=2026-10-19T14:31:08.123Z;msgid=RfcBTzyxY120004 :papo!~papo@localhost PRIVMSG #papo :deploy finished")
        .server("@batch=history;account=emily;time=2026-10-19T14:31:11.456Z;msgid=RfcBTzyxY120005 :emily!~emily@localhost PRIVMSG #papo :looks good")
        .server("@batch=history;time=2026-10-19T14:31:15.789Z;msgid=RfcBTzyxY120006 :papo!~papo@localhost PRIVMSG #papo :from an older server")
        .server("@batch=history;account=papo;time=2026-10-19T14:31:19.012Z;msgid=RfcBTzyxY120007 :papo-old!~papo@localhost NICK papo")
        .server(":ergo.test BATCH -history")
        .event(|event| {
            let Event::History { items, .. } = event else {
                return false;
            };
            matches!(&items[..],
                [HistoryItem::Message(own), HistoryItem::Message(other), HistoryItem::Message(unknown), HistoryItem::NickChanged(changed)]
                if own.authorship == Authorship::Own
                    && other.authorship == Authorship::Other
                    && unknown.authorship == Authorship::Unknown
                    && changed.authorship == Authorship::Own)
        });
}

#[test]
fn history_mode_authorship_distinguishes_actor_and_subject() {
    Transcript::new(Config::default().sasl(Sasl::plain("papo", "correct horse battery staple")))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .server("AUTHENTICATE +")
        .client("AUTHENTICATE AHBhcG8AY29ycmVjdCBob3JzZSBiYXR0ZXJ5IHN0YXBsZQ==")
        .server(":ergo.test 900 papo papo!~papo@localhost papo :You are now logged in as papo")
        .server(":ergo.test 903 papo :SASL authentication successful")
        .client("CAP END")
        .server(":ergo.test BATCH +history chathistory #papo")
        .server("@batch=history;account=papo :papo-old!~papo@localhost MODE #papo +m")
        .server("@batch=history;account=emily :emily!~emily@localhost MODE papo +i")
        .server("@batch=history :papo!~papo@localhost MODE #papo -m")
        .server(":ergo.test BATCH -history")
        .event(|event| {
            let Event::History { items, .. } = event else {
                return false;
            };
            matches!(&items[..],
                [HistoryItem::ModeChanged(own_channel), HistoryItem::ModeChanged(other_user), HistoryItem::ModeChanged(unknown_channel)]
                if own_channel.authorship == Authorship::Own
                    && own_channel.subject_authorship == Authorship::Other
                    && other_user.authorship == Authorship::Other
                    && other_user.subject_authorship == Authorship::Unknown
                    && unknown_channel.authorship == Authorship::Unknown
                    && unknown_channel.subject_authorship == Authorship::Other)
        });
}

#[test]
fn live_mode_and_nick_authorship_distinguish_own_identity() {
    Transcript::new(Config::default())
        .server(":papo!~papo@localhost MODE papo +i")
        .event(|event| {
            matches!(
                event,
                Event::ModeChanged {
                    subject: Subject::User(nick),
                    authorship: Authorship::Own,
                    subject_authorship: Authorship::Own,
                    ..
                } if nick == "papo"
            )
        })
        // A server may define a nonstandard channel prefix. When that prefix makes a
        // channel name equal our nick, the target namespace still decides authorship.
        .server(":ergo.test 005 papo CHANTYPES=p :are supported by this server")
        .server(":papo!~papo@localhost MODE papo +m")
        .event(|event| {
            matches!(
                event,
                Event::ModeChanged {
                    subject: Subject::Channel(channel),
                    authorship: Authorship::Own,
                    subject_authorship: Authorship::Other,
                    ..
                } if channel == "papo"
            )
        })
        .server(":papo-old!~papo@localhost NICK papo")
        .event(|event| {
            matches!(
                event,
                Event::NickChanged {
                    old,
                    new,
                    authorship: Authorship::Other,
                    ..
                } if old == "papo-old" && new == "papo"
            )
        });
}
