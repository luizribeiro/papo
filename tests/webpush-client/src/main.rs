//! Drive typed papo Web Push operations in the NixOS VM test.

use std::{env, error::Error, fmt};

use papo::{
    AccountCreation, Client, Error as PapoError, Event, Feature, Line, NewAccount,
    PersistenceSetting, PushKeys, Refusal, Sasl, Secret,
};

const CHANNEL: &str = "#papo-webpush";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let command = args.next().ok_or("missing command")?;
    let url = args.next().ok_or("missing server URL")?;

    match command.as_str() {
        "create" => {
            let account = next(&mut args, "account")?;
            let password = next(&mut args, "password")?;
            create(&url, &account, &password).await?;
        }
        "register" => {
            let account = next(&mut args, "account")?;
            let password = next(&mut args, "password")?;
            let endpoint = next(&mut args, "endpoint")?;
            let keys = keys(&mut args)?;
            let (client, _events) = authenticated(&url, &account, &password).await?;
            let status = client.persistence().set(PersistenceSetting::On).await?;
            assert!(status.effective);
            if client.snapshot().__channel_local(CHANNEL).is_none() {
                client.join(CHANNEL).await?;
            }
            client.set_away("Web Push VM test").await?;
            let outcome = client.webpush().register(endpoint, keys).await?;
            assert!(!outcome.persistence_required());
            println!("REGISTERED");
        }
        "persistence-required" => {
            let account = next(&mut args, "account")?;
            let password = next(&mut args, "password")?;
            let endpoint = next(&mut args, "endpoint")?;
            let keys = keys(&mut args)?;
            let (client, _events) = authenticated(&url, &account, &password).await?;
            let outcome = client.webpush().register(endpoint, keys).await?;
            assert!(outcome.persistence_required());
            println!("PERSISTENCE_REQUIRED");
        }
        "unregister" => {
            let account = next(&mut args, "account")?;
            let password = next(&mut args, "password")?;
            let endpoint = next(&mut args, "endpoint")?;
            let (client, _events) = authenticated(&url, &account, &password).await?;
            client.webpush().unregister(endpoint).await?;
            println!("UNREGISTERED");
        }
        "forbidden" => {
            let endpoint = next(&mut args, "endpoint")?;
            let keys = keys(&mut args)?;
            let (client, _events) = anonymous(&url, "unregistered-user").await?;
            expect_refusal(
                client.webpush().register(endpoint, keys).await,
                Refusal::WebPushForbidden,
            )?;
            println!("TYPED_FORBIDDEN");
        }
        "limit" => {
            let account = next(&mut args, "account")?;
            let password = next(&mut args, "password")?;
            let endpoint = next(&mut args, "endpoint")?;
            let keys = keys(&mut args)?;
            let (client, _events) = authenticated(&url, &account, &password).await?;
            expect_refusal(
                client.webpush().register(endpoint, keys).await,
                Refusal::WebPushRegistrationLimit,
            )?;
            println!("TYPED_LIMIT");
        }
        "send" => {
            let nick = next(&mut args, "nick")?;
            let target = next(&mut args, "target")?;
            let text = next(&mut args, "text")?;
            let (client, mut events) = anonymous(&url, &nick).await?;
            if target.starts_with('#') {
                client.join(&target).await?;
            }
            client
                .send_raw(Line::new("PRIVMSG").with_params([target, text.clone()]))
                .await?;
            wait_for_echo(&mut events, &text).await?;
            println!("SENT");
        }
        _ => return Err(format!("unknown command: {command}").into()),
    }
    Ok(())
}

async fn create(url: &str, account: &str, password: &str) -> Result<(), Box<dyn Error>> {
    let (client, _events) = anonymous(url, account).await?;
    let outcome = client
        .create_account(NewAccount::new(Secret::new(password)).account(account))
        .await?;
    assert!(matches!(outcome, AccountCreation::Created { .. }));
    println!("CREATED");
    Ok(())
}

async fn anonymous(url: &str, nick: &str) -> Result<(Client, papo::Events), Box<dyn Error>> {
    let connected = Client::builder(url, nick)
        .allow_insecure_auth(true)
        .enable(Feature::WebPush)
        .enable(Feature::Persistence)
        .connect()
        .await?;
    Ok(connected)
}

async fn authenticated(
    url: &str,
    account: &str,
    password: &str,
) -> Result<(Client, papo::Events), Box<dyn Error>> {
    let (client, mut events) = Client::builder(url, account)
        .allow_insecure_auth(true)
        .sasl(Sasl::plain(account, Secret::new(password)))
        .enable(Feature::WebPush)
        .enable(Feature::Persistence)
        .connect()
        .await?;
    while let Some(event) = events.next().await.map(|(event, _)| event) {
        match event {
            Event::Connected(_) => return Ok((client, events)),
            Event::Disconnected(disconnected) => {
                return Err(format!("disconnected during login: {disconnected}").into());
            }
            _ => {}
        }
    }
    Err("event stream ended during login".into())
}

fn keys(args: &mut impl Iterator<Item = String>) -> Result<PushKeys, Box<dyn Error>> {
    Ok(PushKeys::new(
        next(args, "p256dh")?,
        Secret::new(next(args, "auth")?),
    ))
}

fn next(args: &mut impl Iterator<Item = String>, name: &str) -> Result<String, Box<dyn Error>> {
    args.next().ok_or_else(|| format!("missing {name}").into())
}

fn expect_refusal<T: fmt::Debug>(
    result: Result<T, PapoError>,
    expected: Refusal,
) -> Result<(), Box<dyn Error>> {
    match result {
        Err(PapoError::Server(error)) if error.refusal() == expected => Ok(()),
        other => Err(format!("expected {expected:?}, got {other:?}").into()),
    }
}

async fn wait_for_echo(events: &mut papo::Events, text: &str) -> Result<(), Box<dyn Error>> {
    while let Some(event) = events.next().await.map(|(event, _)| event) {
        match event {
            Event::Message(message) if message.text == text => return Ok(()),
            Event::Disconnected(disconnected) => {
                return Err(format!("disconnected before echo: {disconnected}").into());
            }
            _ => {}
        }
    }
    Err("event stream ended before echo".into())
}
