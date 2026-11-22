use crate::driver::test_support::*;
use crate::event::HistoryPage;
use crate::{AccountStatus, Client, Events};

const CAPS: &str = "account-tag batch draft/chathistory draft/message-redaction extended-join labeled-response message-tags server-time";
const TAGGED: &str = "pwcbkkexfp2uqmk5ri5x5b5bya";
const UNTAGGED: &str = "6e7q9yyfrx3nwsw2i6yjzmzxyc";
const SOJU_ID: &str = "rtaqwb2dxm8u85ap38mvhqxy62";

const ERGO_PAGE: &[u8] = b"@label=0 :ergo.test BATCH +reply labeled-response\r\n\
@batch=reply;draft/chathistory-end :ergo.test BATCH +h chathistory writer\r\n\
@batch=h;time=2026-11-01T09:00:00.000Z;msgid=6e7q9yyfrx3nwsw2i6yjzmzxyc :papo-driver!~u@papo.irc PRIVMSG writer :is the release ready?\r\n\
@batch=h;time=2026-11-01T09:01:00.000Z;msgid=pwcbkkexfp2uqmk5ri5x5b5bya;account=writer :writer!~u@writer.irc PRIVMSG papo-driver :the release is tagged\r\n\
@batch=reply :ergo.test BATCH -h\r\n\
:ergo.test BATCH -reply\r\n";

const SOJU_PAGE: &[u8] = b"@label=0 BATCH +reply labeled-response\r\n\
@batch=reply BATCH +history chathistory writer\r\n\
@time=2026-11-12T05:21:06.731Z;msgid=rtaqwb2dxm8u85ap38mvhqxy62;batch=history :writer!~u@mhqynk9w52dpq.irc PRIVMSG reader :can you send the incident report?\r\n\
@batch=reply BATCH -history\r\n\
BATCH -reply\r\n";

async fn connect() -> (
    DuplexStream,
    Client,
    Events,
    tokio::task::JoinHandle<Result<(), Error>>,
) {
    let (mut server, client, mut events, driver) = scripted(config().wanted_caps(CAPS.split(' ')));
    let task = tokio::spawn(driver);
    complete_connect_caps(&mut server, &mut events, CAPS, CAPS).await;
    (server, client, events, task)
}

async fn fetch(server: &mut DuplexStream, client: &Client, transcript: &[u8]) -> HistoryPage {
    let client = client.clone();
    let request = tokio::spawn(async move { client.history("writer").latest().limit(10).await });
    client_line(server, b"@label=0 CHATHISTORY LATEST writer * 10\r\n").await;
    server_line(server, transcript).await;
    within(request).await.unwrap().unwrap()
}

async fn fence(server: &mut DuplexStream) -> Vec<u8> {
    server_line(server, b"PING :history-fence\r\n").await;
    next_client_line(server).await
}

async fn expect_send(server: &mut DuplexStream, expected: &str) {
    let line = String::from_utf8(next_client_line(server).await).unwrap();
    assert_eq!(line, expected);
}

#[tokio::test(flavor = "current_thread")]
async fn private_history_references_address_the_thread_peer_without_who() {
    for (transcript, expected_ids) in [
        (ERGO_PAGE, &[UNTAGGED, TAGGED][..]),
        (SOJU_PAGE, &[SOJU_ID]),
    ] {
        let (mut server, client, mut events, task) = connect().await;
        let page = fetch(&mut server, &client, transcript).await;
        let messages: Vec<_> = page.chat_messages().collect();
        assert_eq!(messages.len(), expected_ids.len());
        for (index, (message, id)) in messages.into_iter().zip(expected_ids).enumerate() {
            assert_eq!(message.msgid().unwrap().as_ref(), *id);
            let reply = tokio::spawn(client.reply(message, "thanks for the update").into_future());
            expect_send(
                &mut server,
                &format!("@+reply={id} PRIVMSG writer :thanks for the update\r\n"),
            )
            .await;
            within(reply).await.unwrap().unwrap();
            let mut label = 2 + 4 * index;
            for (request, tag) in [
                (client.react(message, "✅"), "+draft/react"),
                (client.unreact(message, "✅"), "+draft/unreact"),
            ] {
                let reaction = tokio::spawn(request);
                expect_send(
                    &mut server,
                    &format!("@+reply={id};{tag}=✅;label={label} TAGMSG writer\r\n"),
                )
                .await;
                server_line(
                    &mut server,
                    format!("@label={label} :ergo.test ACK\r\n").as_bytes(),
                )
                .await;
                within(reaction).await.unwrap().unwrap();
                label += 1;
            }
            let redaction = tokio::spawn(client.redact(message, "superseded"));
            expect_send(
                &mut server,
                &format!("@label={label} REDACT writer {id} superseded\r\n"),
            )
            .await;
            server_line(
                &mut server,
                format!("@label={label} :ergo.test ACK\r\n").as_bytes(),
            )
            .await;
            within(redaction).await.unwrap().unwrap();
        }

        // The saved reference predates the local nickname change.
        server_line(&mut server, b":papo-driver!~u@papo.irc NICK WRITER\r\n").await;
        loop {
            if matches!(within(next_event(&mut events)).await.unwrap(), Event::NickChanged(change) if change.new == "WRITER")
            {
                break;
            }
        }
        let message = page.chat_messages().last().unwrap();
        assert!(matches!(
            within(client.reply(message, "status update")).await,
            Err(Error::TargetGone)
        ));
        assert_eq!(
            fence(&mut server).await.as_slice(),
            b"PONG history-fence\r\n"
        );
        abort_task(task).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn private_history_actor_refuses_a_direct_send_even_with_a_matching_live_account() {
    let (mut server, client, _events, task) = connect().await;
    server_line(
        &mut server,
        b":papo-driver!~u@papo.irc JOIN #release * :Papo\r\n\
:writer!~u@writer.irc JOIN #release writer :Release Writer\r\n",
    )
    .await;
    assert_eq!(
        fence(&mut server).await.as_slice(),
        b"PONG history-fence\r\n"
    );

    let page = fetch(&mut server, &client, ERGO_PAGE).await;
    let archived = page.chat_messages().last().unwrap();
    assert!(matches!(archived.actor_account(), AccountStatus::LoggedIn(name) if name == "writer"));
    assert!(matches!(
        within(client.privmsg(&archived.actor, "the next release is scheduled")).await,
        Err(Error::TargetGone)
    ));
    assert_eq!(
        fence(&mut server).await.as_slice(),
        b"PONG history-fence\r\n"
    );
    abort_task(task).await;
}
