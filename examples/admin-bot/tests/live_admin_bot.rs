//! Admin bot behavior against the runner-managed Ergo server.
#![cfg(not(target_family = "wasm"))]

mod support;

use std::time::Duration;

use papo::MessageKind;
use papo_test_server::{create_account, from_env, unique};
use support::{
    AdminBot, PASSWORD, connect, wait_join, wait_message, wait_message_starting_with, wait_op,
};
use tokio::time::{sleep, timeout};

#[tokio::test(flavor = "current_thread")]
async fn parses_mentions_and_authorizes_by_account() {
    timeout(Duration::from_secs(25), async {
        let server = from_env();
        let bot_nick = unique("papo-admin-bot");
        let admin_nick = unique("papo-admin");
        let user_nick = unique("papo-user");
        let channels = vec![format!("#{}", unique("papo-admin"))];
        create_account(&server, &admin_nick, PASSWORD).unwrap();
        let _bot = AdminBot::start(&server, &bot_nick, &admin_nick, &channels[0]);
        sleep(Duration::from_millis(500)).await;

        let (user, mut user_events) = connect(&server, &user_nick, &channels, false).await;
        let (admin, mut admin_events) = connect(&server, &admin_nick, &channels, true).await;

        let ping = format!("{bot_nick}: ping");
        let sent = user.privmsg(&channels[0], &ping).await.unwrap();
        let msgid = sent.msgid.expect("Ergo supplies msgid");
        let response = wait_message(&mut user_events, "pong").await;
        assert_eq!(response.reply_to.as_ref(), Some(&msgid));

        user.privmsg(&channels[0], format!("!op {admin_nick}"))
            .await
            .unwrap();
        let refusal = wait_message(
            &mut user_events,
            &format!("cannot op {admin_nick}: permission denied"),
        )
        .await;
        assert_eq!(refusal.kind, MessageKind::Privmsg);
        assert!(matches!(
            refusal.conversation,
            papo::Conversation::Channel(_)
        ));

        admin
            .privmsg(&channels[0], format!("!op {user_nick}"))
            .await
            .unwrap();
        wait_op(&mut admin_events, &channels[0], &user_nick).await;

        user.quit("admin bot test complete").await.unwrap();
        admin.quit("admin bot test complete").await.unwrap();
    })
    .await
    .expect("live admin bot test did not finish within 25 seconds");
}

#[tokio::test(flavor = "current_thread")]
async fn keeps_running_after_action_refusals() {
    timeout(Duration::from_secs(20), async {
        let server = from_env();
        let bot_nick = unique("papo-admin-bot");
        let admin_nick = unique("papo-admin");
        let missing_nick = unique("papo-missing");
        let channels = vec![format!("#{}", unique("papo-admin"))];
        create_account(&server, &admin_nick, PASSWORD).unwrap();
        let (admin, mut events) = connect(&server, &admin_nick, &channels, true).await;
        let bot = AdminBot::start(&server, &bot_nick, &admin_nick, &channels[0]);
        wait_join(&mut events, &bot_nick).await;

        admin
            .privmsg(&channels[0], format!("!op {missing_nick}"))
            .await
            .unwrap();
        let prefix = format!("cannot op {missing_nick}: ");
        let refusal = wait_message_starting_with(&mut events, &prefix).await;
        assert!(refusal.text.len() > prefix.len());
        admin.privmsg(&channels[0], "!ping").await.unwrap();
        wait_message(&mut events, "pong").await;

        // Reset the default limit of two replies per five-second window.
        sleep(Duration::from_millis(5_100)).await;
        admin.mode(&channels[0]).add('m').await.unwrap();
        admin.privmsg(&channels[0], "!ping").await.unwrap();
        assert!(
            bot.next_error()
                .starts_with("could not reply: server refusal 404:")
        );
        admin.mode(&channels[0]).remove('m').await.unwrap();
        admin.privmsg(&channels[0], "!ping").await.unwrap();
        wait_message(&mut events, "pong").await;

        admin.quit("admin bot test complete").await.unwrap();
    })
    .await
    .expect("live admin bot refusal test did not finish within 20 seconds");
}
