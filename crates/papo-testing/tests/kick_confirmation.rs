//! Highest-layer regressions for which `KICK` confirms our own kick.

mod support {
    pub mod direct;
}

use papo::{Error, Reconnect, Refusal};
use papo_testing::wire;
use support::direct::{connected, next_active_event};

const CHANOP_KICK: &str = ":alice!~alice@cloak.example KICK #review mallory :ban evasion";

enum Reply {
    Refused,
    Echoed,
}

async fn kick_while_another_operator_kicks(caps: &[&str], reply: Reply) -> Result<(), Error> {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(caps).await;
    connected(&mut events).await;
    for line in [
        ":papo!~papo@localhost JOIN #review",
        ":alice!~alice@cloak.example JOIN #review",
        ":mallory!~mallory@users.example JOIN #review",
    ] {
        wire.server(line).await;
        assert!(matches!(
            next_active_event(&mut events).await,
            papo::Event::Joined(_)
        ));
    }

    let kicker = client.clone();
    let kick = tokio::spawn(async move { kicker.kick("#review", "mallory").reason("spam").await });
    let sent = wire.next_sent().await;
    let (tags, command) = match sent.strip_prefix('@') {
        Some(tagged) => tagged.split_once(' ').unwrap(),
        None => ("", sent.as_str()),
    };
    assert_eq!(command, "KICK #review mallory spam");
    let label = if tags.is_empty() {
        String::new()
    } else {
        format!("@{tags} ")
    };

    wire.server(CHANOP_KICK).await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::Kicked(_)
    ));
    wire.server(match reply {
        Reply::Refused => {
            format!("{label}:offline.test 482 papo #review :You're not a channel operator")
        }
        Reply::Echoed => format!("{label}:PAPO!~papo@localhost KICK #review mallory :spam"),
    })
    .await;
    kick.await.unwrap().map(drop)
}

#[tokio::test]
async fn another_operators_kick_never_confirms_ours() {
    for caps in [&[][..], &["labeled-response", "batch", "message-tags"][..]] {
        let refused = kick_while_another_operator_kicks(caps, Reply::Refused).await;
        assert!(
            matches!(
                &refused,
                Err(Error::Server(error))
                    if error.refusal() == Refusal::ChannelOperatorPrivilegesNeeded
            ),
            "{caps:?}: {refused:?}"
        );
        let echoed = kick_while_another_operator_kicks(caps, Reply::Echoed).await;
        assert!(echoed.is_ok(), "{caps:?}: {echoed:?}");
    }
}
