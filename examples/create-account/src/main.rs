//! Create an IRC account while connecting.

use std::{env, error::Error, io};

use papo::{AccountCreation, Client, Event, NewAccount};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = env::args()
        .nth(1)
        .map_or_else(|| env::var("IRC_SERVER"), Ok)?;
    let nick = env::var("IRC_NICK")?;
    let mut account = NewAccount::new(env::var("IRC_PASSWORD")?).account(&nick);
    if let Ok(email) = env::var("IRC_EMAIL") {
        account = account.email(email);
    }
    let (client, mut events) = Client::builder(server)
        .nick(&nick)
        .create_account(account)
        .connect()
        .await?;

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
