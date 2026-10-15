//! Admin bot behavior against the runner-managed Ergo server.
#![cfg(not(target_family = "wasm"))]

mod support;

use std::time::Duration;

use papo::MessageKind;
use papo_test_server::{from_env, register_account, unique};
use support::{AdminBot, PASSWORD, connect, wait_message, wait_op};
use tokio::time::{sleep, timeout};

#[tokio::test(flavor = "current_thread")]
async fn routes_authorizes_contextualizes_and_limits_commands() {
    timeout(Duration::from_secs(25), async {
        let server = from_env();
        let bot_nick = unique("papo-admin-bot");
        let admin_nick = unique("papo-admin");
        let user_nick = unique("papo-user");
        let channels: Vec<_> = ["commands", "mentions", "operators", "rate"]
            .map(|kind| format!("#{}-{kind}", unique("papo-admin")))
            .into();
        register_account(&server, &bot_nick, PASSWORD).unwrap();
        register_account(&server, &admin_nick, PASSWORD).unwrap();
        let mut bot = AdminBot::start(&server, &bot_nick, &channels.join(","), &admin_nick);
        sleep(Duration::from_millis(500)).await;

        let (user, mut user_events) = connect(&server, &user_nick, &channels, false).await;
        let (admin, mut admin_events) = connect(&server, &admin_nick, &channels, true).await;

        user.privmsg(&channels[0], "!ping").await.unwrap();
        let sent = wait_message(&mut user_events, "!ping").await;
        let msgid = sent.msgid().expect("Ergo supplies msgid").to_owned();
        let response = wait_message(&mut user_events, "pong").await;
        assert_eq!(response.reply_to(), Some(msgid.as_str()));

        user.privmsg(&channels[0], "!echo 'Ergo chat'")
            .await
            .unwrap();
        assert_eq!(
            wait_message(&mut user_events, "Ergo chat").await.text(),
            "Ergo chat"
        );

        user.privmsg(&channels[1], &format!("{bot_nick}: ping"))
            .await
            .unwrap();
        assert_eq!(
            wait_message(&mut user_events, "pong").await.target(),
            channels[1]
        );

        user.privmsg(&channels[2], &format!("!op {admin_nick}"))
            .await
            .unwrap();
        let refusal = wait_message(
            &mut user_events,
            "permission denied: authenticated account required",
        )
        .await;
        assert_eq!(refusal.kind(), MessageKind::Notice);
        assert!(refusal.is_private());
        let raw = refusal.as_ref();
        let context = raw
            .tag("+channel-context")
            .or_else(|| raw.tag("+draft/channel-context"));
        assert_eq!(context.as_deref(), Some(channels[2].as_str()));

        admin
            .privmsg(&channels[2], &format!("!op {user_nick}"))
            .await
            .unwrap();
        wait_op(&mut admin_events, &channels[2], &user_nick).await;

        user.privmsg(&bot_nick, "!echo private command")
            .await
            .unwrap();
        let private = wait_message(&mut user_events, "private command").await;
        assert!(private.is_private());

        for _ in 0..3 {
            user.privmsg(&channels[3], "!ping").await.unwrap();
        }
        wait_message(&mut user_events, "pong").await;
        wait_message(&mut user_events, "pong").await;
        assert!(
            timeout(
                Duration::from_millis(300),
                wait_message(&mut user_events, "pong")
            )
            .await
            .is_err()
        );
        sleep(Duration::from_secs(5)).await;
        user.privmsg(&channels[3], "!ping").await.unwrap();
        wait_message(&mut user_events, "pong").await;

        bot.shutdown().await;
        user.quit(Some("admin bot test complete")).await.unwrap();
        admin.quit(Some("admin bot test complete")).await.unwrap();
    })
    .await
    .expect("live admin bot test did not finish within 25 seconds");
}
