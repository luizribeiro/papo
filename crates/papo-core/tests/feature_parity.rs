//! Differential coverage for stable behavior with optional API compiled in.

use std::time::Instant;

use papo_core::{Config, Event, RateLimit, ServerError, Session};
use papo_proto::Line;

const TRAFFIC: &[&str] = &[
    ":ergo.test CAP * LS :account-tag batch draft/chathistory draft/multiline labeled-response server-time standard-replies",
    ":ergo.test CAP * ACK :batch labeled-response",
    "@label=0 :ergo.test 401 papo missing :No such nick/channel",
    ":ergo.test 001 papo :Welcome to the ErgoTest IRC Network papo",
    ":ergo.test 005 papo CASEMAPPING=ascii NETWORK=ErgoTest PREFIX=(qaohv)~&@%+ UTF8ONLY :are supported by this server",
    ":papo!papo@127.0.0.1 JOIN #papo",
    ":alice!~alice@cloak.example JOIN #papo * :Alice Example",
    "@time=2026-10-15T12:00:00.000Z;msgid=ErgoMessage42 :alice!~alice@cloak.example PRIVMSG #papo :hello from Ergo",
    ":solanum.example 005 papo ACCOUNTEXTBAN=a CHANMODES=eIbq,k,flj,CFLPQcgimnprstz EXTBAN=$,a PREFIX=(ov)@+ :are supported",
    ":irc.inspircd.example 005 papo ACCOUNTEXTBAN=account,R CHANMODES=b,k,l,MRimnprst EXTBAN=,RU PREFIX=(Yov)!@+ :are supported",
    ":irc.inspircd.example 353 papo = #papo :@papo !service +alice",
    ":irc.inspircd.example 366 papo #papo :End of /NAMES list",
    ":irc.unreal.test 442 papo #services :You're not on that channel",
    ":soju.example BATCH +search-42 soju.im/search",
    "@batch=search-42 :alice!~a@client.example PRIVMSG #papo :release is ready",
    ":soju.example BATCH -search-42",
    ":soju.example BATCH +networks soju.im/bouncer-networks",
    "@batch=networks BOUNCER NETWORK libera name=Libera\\sChat;state=connected",
    ":soju.example BATCH -networks",
    ":soju.example FAIL BOUNCER INVALID_NETID libera :Unknown network ID",
    ":ergo.test FAIL RELAYMSG NOT_ENABLED #bridged :Relay messaging is disabled",
    ":ergo.test FAIL PERSISTENCE INVALID_PARAMS :Invalid persistence parameters",
    ":ergo.test FAIL WEBPUSH MAX_REGISTRATIONS :Too many registrations",
];

fn event_debug(event: &Event) -> String {
    #[allow(unused_mut)]
    let mut output = format!("{event:?}");
    #[cfg(feature = "unstable")]
    if matches!(event, Event::StandardReply(_))
        && let Some(start) = output.rfind(", refusal: ")
    {
        output.replace_range(start..output.len() - 3, "");
    }
    output
}

fn state_debug(session: &Session) -> String {
    let state = session.state();
    let mut tokens = session
        .isupport()
        .tokens()
        .map(|(name, value)| format!("{name}={value:?}"))
        .collect::<Vec<_>>();
    tokens.sort();
    let channels = state
        .channels()
        .map(|channel| {
            let mut members = channel
                .members()
                .map(|member| (member.nick().to_string(), member.prefixes().to_vec()))
                .collect::<Vec<_>>();
            members.sort_by(|left, right| left.0.cmp(&right.0));
            (channel.name().to_string(), channel.is_synced(), members)
        })
        .collect::<Vec<_>>();
    let users = ["papo", "alice", "service"]
        .into_iter()
        .filter_map(|nick| {
            state.user(nick).map(|user| {
                (
                    user.nick().to_string(),
                    user.account().clone(),
                    user.away().clone(),
                )
            })
        })
        .collect::<Vec<_>>();
    format!(
        "nick={:?} user={:?} host={:?} account={:?} away={:?} casemapping={:?} isupport={tokens:?} channels={channels:?} users={users:?} monitored={:?} metadata_targets={} last_seen={:?}",
        state.nick(),
        state.username(),
        state.host(),
        state.account(),
        state.away(),
        state.casemapping(),
        state
            .monitored()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        state.metadata_target_count(),
        state.last_seen("#papo"),
    )
}

fn drain(session: &mut Session) {
    while let Some(bytes) = session.poll_transmit() {
        println!("CLIENT {}", String::from_utf8_lossy(&bytes).trim_end());
    }
    while let Some(event) = session.poll_event() {
        println!("EVENT {}", event_debug(&event));
        if let Event::StandardReply(reply) = &event
            && reply.refusal().is_some()
        {
            let error = ServerError::__standard(
                reply.command.clone(),
                reply.code.clone(),
                reply.context.clone(),
                reply.description.clone(),
            );
            println!("STANDARD_REFUSAL {:?}", reply.refusal());
            println!("SERVER_REFUSAL {:?}", error.refusal());
        }
        if let Event::Response(response) = &event
            && let Some(error) = response.error()
        {
            println!("SERVER_REFUSAL {:?}", error.refusal());
        }
    }
}

#[test]
fn disabled_extensions_match_stable_behavior() {
    let now = Instant::now();
    let mut config = Config::default();
    config.rate_limit = RateLimit::unlimited();
    let mut session = Session::new(config);

    println!("PAPO_FEATURE_PARITY_BEGIN");
    session.handle_connected(now);
    drain(&mut session);
    for line in TRAFFIC {
        println!("SERVER {line}");
        session.handle_line(now, line.as_bytes());
        drain(&mut session);
        if line.contains("CAP * ACK") {
            session
                .request_silent(now, Line::new("WHOIS").with_params(["missing"]))
                .unwrap();
            drain(&mut session);
        }
        println!("STATE {}", state_debug(&session));
    }
    println!("PAPO_FEATURE_PARITY_END");
}
