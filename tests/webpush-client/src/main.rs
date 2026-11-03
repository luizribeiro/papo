//! Drive typed papo Web Push operations in the NixOS VM test.

use std::{env, error::Error};

use papo::{
    AccountCreation, Client, Event, Feature, NewAccount, PersistenceSetting, PushKeys, Sasl, Secret,
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
            if client.state().channel(CHANNEL).is_none() {
                client.join(CHANNEL).await?;
            }
            client.set_away("Web Push VM test").await?;
            client.webpush().register(endpoint, keys).await?;
            println!("REGISTERED");
        }
        "unregister" => {
            let account = next(&mut args, "account")?;
            let password = next(&mut args, "password")?;
            let endpoint = next(&mut args, "endpoint")?;
            let (client, _events) = authenticated(&url, &account, &password).await?;
            client.webpush().unregister(endpoint).await?;
            println!("UNREGISTERED");
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
    while let Some(event) = events.next().await {
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
