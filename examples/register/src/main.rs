//! Register an Ergo account before completing connection registration.

use std::{env, error::Error, io};

use papo::{Client, Event};
use papo_core::Secret;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = required("IRC_SERVER")?;
    let account = required("IRC_ACCOUNT")?;
    let password = std::fs::read_to_string(required("IRC_PASSWORD_FILE")?)?;
    let email = env::var("IRC_EMAIL").ok();
    let mut builder = Client::builder(&server).nick(&account).register_on_connect(
        Some(&account),
        email.as_deref(),
        Secret::new(password.trim_end_matches(['\r', '\n'])),
    );
    if let Some(ca_file) = env::var_os("IRC_CA_FILE") {
        builder = builder.tls_ca_file(ca_file)?;
    }
    let (client, mut events, driver) = builder.build();
    tokio::spawn(driver);

    while let Some(event) = events.next().await {
        match event {
            Event::VerificationRequired { account, message } => {
                eprintln!("{message}");
                let mut code = String::new();
                io::stdin().read_line(&mut code)?;
                client.verify(&account, code.trim()).await?;
            }
            Event::Registered(_) => {
                let account = client
                    .state(|state| state.account().map(str::to_owned))
                    .ok_or("server registered the connection without logging in")?;
                println!("logged in as {account}");
                client.quit(Some("account registration complete")).await?;
                return Ok(());
            }
            Event::Disconnected { reason, .. } => return Err(reason.into()),
            _ => {}
        }
    }
    Err("driver stopped before registration completed".into())
}

fn required(name: &str) -> io::Result<String> {
    env::var(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, format!("{name} is required")))
}
