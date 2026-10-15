//! Run an authenticated command bot with account-based operator access.

use std::{env, error::Error, io};

use papo::{Client, Sasl, bot::Bot};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = required("IRC_SERVER")?;
    let nick = required("IRC_NICK")?;
    let password_file = required("IRC_PASSWORD_FILE")?;
    let channels = required("IRC_CHANNELS")?;
    let _admins = required("IRC_ADMINS")?;
    let password = std::fs::read_to_string(password_file)?;

    let mut builder = Client::builder(&server)
        .nick(&nick)
        .sasl(Sasl::plain(&nick, password.trim_end_matches(['\r', '\n'])))
        .require_sasl(true)
        .bot()
        .channels(channels.split(',').map(str::trim));
    if let Some(ca_file) = env::var_os("IRC_CA_FILE") {
        builder = builder.tls_ca_file(ca_file)?;
    }
    let (client, events) = builder.connect().await?;
    Bot::new(client).prefix("!").run(events).await?;
    Ok(())
}

fn required(name: &str) -> io::Result<String> {
    env::var(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, format!("{name} is required")))
}
