use crate::driver::test_support::*;

#[tokio::test(flavor = "current_thread")]
async fn a_send_through_a_handle_reports_the_membership_it_was_sent_through() {
    let (mut server, client, mut events, driver) = scripted(config());
    let task = tokio::spawn(driver);
    complete_connect(&mut server, &mut events).await;
    let join = tokio::spawn({
        let client = client.clone();
        async move { client.join("#papo").await }
    });
    client_line(&mut server, b"JOIN #papo\r\n").await;
    server_line(
        &mut server,
        b":papo-driver!~u@7z29yeuh6n8ta.irc JOIN #papo\r\n:ergo.test 353 papo-driver = #papo :@papo-driver\r\n:ergo.test 366 papo-driver #papo :End of NAMES list\r\n",
    )
    .await;
    let handle = within(join).await.unwrap().unwrap();
    client_line(&mut server, b"MODE #papo\r\n").await;
    client_line(&mut server, b"WHO #papo\r\n").await;
    server_line(
        &mut server,
        b":ergo.test 324 papo-driver #papo +Cnt\r\n:ergo.test 329 papo-driver #papo 1763228532\r\n:ergo.test 315 papo-driver #papo :End of WHO list\r\nPING :synced\r\n",
    )
    .await;
    client_line(&mut server, b"PONG synced\r\n").await;
    let sent = tokio::spawn({
        let client = client.clone();
        let handle = handle.clone();
        async move { client.privmsg(&handle, "deploy finished").await }
    });
    client_line(&mut server, b"PRIVMSG #papo :deploy finished\r\n").await;
    let sent = within(sent).await.unwrap().unwrap();
    assert_eq!(sent.conversation, crate::Conversation::Channel(handle));
    task.abort();
}
