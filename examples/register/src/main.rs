//! Register an Ergo account before completing connection registration.

use std::{env, error::Error, io};

use papo::{Client, Endpoint, Event};
use papo_core::Secret;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = required("IRC_SERVER")?;
    let account = required("IRC_ACCOUNT")?;
    let password = std::fs::read_to_string(required("IRC_PASSWORD_FILE")?)?;
    let email = env::var("IRC_EMAIL").ok();
    let mut builder = Client::builder(endpoint(server)?)
        .nick(&account)
        .request_caps(["draft/account-registration"])
        .register_on_connect(
            Some(&account),
            email.as_deref(),
            Secret::new(password.trim_end_matches(['\r', '\n'])),
        );
    if let Some(ca_file) = env::var_os("IRC_CA_FILE") {
        builder = builder.tls_ca_pem(std::fs::read(ca_file)?);
    }
    let (client, mut events, driver) = builder.build();
    tokio::spawn(driver);

    let mut registered = false;
    let mut logged_in = None;
    while let Some(event) = events.next().await {
        match event {
            Event::VerificationRequired { account, message } => {
                eprintln!("{message}");
                let mut code = String::new();
                io::stdin().read_line(&mut code)?;
                client.verify(&account, code.trim()).await?;
            }
            Event::AccountRegistered {
                account,
                logged_in: true,
            } => logged_in = Some(account),
            Event::Registered(_) => registered = true,
            Event::Disconnected { reason, .. } => return Err(reason.into()),
            _ => {}
        }
        if registered && let Some(account) = &logged_in {
            println!("logged in as {account}");
            client.quit(Some("account registration complete")).await?;
            return Ok(());
        }
    }
    Err("driver stopped before registration completed".into())
}

fn endpoint(host: String) -> Result<Endpoint, Box<dyn Error>> {
    let port = env::var("IRC_PORT").map_or(Ok(6697), |value| value.parse())?;
    Ok(if env::var_os("IRC_PLAINTEXT").is_some() {
        Endpoint::plaintext(host, port)
    } else {
        Endpoint::tls(host, port)
    })
}

fn required(name: &str) -> io::Result<String> {
    env::var(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, format!("{name} is required")))
}
