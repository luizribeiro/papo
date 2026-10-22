//! Create an IRC account while connecting.

use std::{env, error::Error, io};

use papo::{AccountCreation, Client, Event, NewAccount};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let url = env::args().nth(1).ok_or("usage: create-account URL")?;
    let nick = env::var("IRC_NICK")?;
    let mut account = NewAccount::new(env::var("IRC_PASSWORD")?).account(&nick);
    if let Ok(email) = env::var("IRC_EMAIL") {
        account = account.email(email);
    }
    let mut builder = Client::builder(url).nick(&nick).create_account(account);
    if env::var_os("IRC_ALLOW_INSECURE_AUTH").is_some() {
        builder = builder.allow_insecure_auth(true);
    }
    let (client, mut events) = builder.connect().await?;

    while let Some(event) = events.next().await {
        let created = match event {
            Event::AccountCreation(AccountCreation::Created { account, .. }) => account,
            Event::AccountCreation(AccountCreation::VerificationRequired {
                account,
                message,
                ..
            }) => {
                eprintln!("{message}");
                let mut code = String::new();
                io::stdin().read_line(&mut code)?;
                match client.verify_account(account, code.trim()).await {
                    Ok(AccountCreation::Created { account, .. }) => account,
                    Ok(_) => continue,
                    Err(error) => {
                        eprintln!("could not verify the account: {error}");
                        continue;
                    }
                }
            }
            _ => continue,
        };
        println!("logged in as {created}");
        return Ok(());
    }
    Ok(())
}
