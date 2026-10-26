//! Persist `IRCv3` STS policies in an application-owned file.

mod file_store;

use std::{env, error::Error, time::UNIX_EPOCH};

use file_store::FileStore;
use papo::{Client, sts::StsPolicy, sts::StsStore};

fn main() -> Result<(), Box<dyn Error>> {
    let url = env::args().nth(1).ok_or("usage: sts-store URL PATH")?;
    let path = env::args_os().nth(2).ok_or("usage: sts-store URL PATH")?;
    let mut store = FileStore::open(&path)?;
    store.put(StsPolicy::new(
        "irc.example.net",
        6697,
        UNIX_EPOCH + std::time::Duration::from_secs(4_000_000_000),
        std::time::Duration::from_hours(4_320),
    ));
    let store = FileStore::open(path)?;
    let policy = store.get("irc.example.net").ok_or("policy was not saved")?;
    println!("{} requires TLS on port {}", policy.host, policy.port);
    let _builder = Client::builder(url, "papo-client").sts_store(store);
    Ok(())
}
