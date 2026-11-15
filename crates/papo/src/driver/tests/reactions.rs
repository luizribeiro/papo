use crate::driver::test_support::*;
use crate::{Confirmation, ReactOutcome};

const ECHO_CAPS: &str = "echo-message labeled-response message-tags";
const ORIGINAL: &str = "3gcfszmd5bzyy9g2evchgjqiv2";

async fn react(caps: &str, tag: &str, address: &str, reply: &str) -> Result<ReactOutcome, Error> {
    let (mut server, client, mut events, driver) = scripted(config().wanted_caps(caps.split(' ')));
    let task = tokio::spawn(driver);
    complete_connect_caps(&mut server, &mut events, caps, caps).await;
    server_line(
        &mut server,
        b":ergo.test 005 papo-driver STATUSMSG=~&@%+ PREFIX=(qaohv)~&@%+ :are supported by this server\r\n",
    )
    .await;
    let message = line_reference(address, ORIGINAL);
    let reacted = tokio::spawn(match tag {
        "react" => client.react(message, "👍"),
        _ => client.unreact(message, "👍"),
    });
    let label = if caps.contains(crate::cap::LABELED_RESPONSE.as_str()) {
        ";label=0"
    } else {
        ""
    };
    let sent = format!("@+reply={ORIGINAL};+draft/{tag}=👍{label} TAGMSG {address}\r\n");
    client_line(&mut server, sent.as_bytes()).await;
    if !reply.is_empty() {
        server_line(&mut server, format!("{reply}\r\n").as_bytes()).await;
    }
    let result = within(reacted).await.unwrap();
    task.abort();
    result
}

fn confirmed(msgid: &str, server_time: &str) -> Confirmation {
    Confirmation::Confirmed {
        msgid: msgid.into(),
        server_time: Some(server_time.parse().unwrap()),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn an_ergo_echo_confirms_a_reaction_to_its_audience() {
    let cases = [
        (
            "react",
            "#papo",
            "@msgid=ripxp4ezdb76is6ep47ic7rexa;time=2026-11-15T17:42:12.749Z;label=0;+draft/reply=3gcfszmd5bzyy9g2evchgjqiv2;+draft/react=👍 :papo-driver!~u@7z29yeuh6n8ta.irc TAGMSG #papo",
            None,
            confirmed("ripxp4ezdb76is6ep47ic7rexa", "2026-11-15T17:42:12.749Z"),
        ),
        (
            "react",
            "@#papo",
            "@time=2026-11-15T17:42:14.751Z;label=0;msgid=6b4b6emqdux5z7beb724swiqhn;+draft/reply=3gcfszmd5bzyy9g2evchgjqiv2;+draft/react=👍 :papo-driver!~u@7z29yeuh6n8ta.irc TAGMSG @#papo",
            Some('@'),
            confirmed("6b4b6emqdux5z7beb724swiqhn", "2026-11-15T17:42:14.751Z"),
        ),
    ];
    for (tag, address, echo, status, confirmation) in cases {
        let outcome = react(ECHO_CAPS, tag, address, echo).await.unwrap();
        assert_eq!(outcome.confirmation, confirmation, "{echo}");
        assert_eq!(outcome.status.map(crate::Prefix::symbol), status, "{echo}");
        assert_eq!(outcome.conversation.to_string(), "#papo", "{echo}");
    }
}
