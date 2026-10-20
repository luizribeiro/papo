//! Persist `IRCv3` STS policies in an application-owned file.

use std::{
    collections::HashMap,
    env,
    error::Error,
    fmt::Write as _,
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

use papo::sts::{StsPolicy, StsStore};

struct FileStore {
    path: PathBuf,
    policies: HashMap<String, StsPolicy>,
}

impl FileStore {
    fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error),
        };
        let policies = text.lines().map(parse_policy).collect::<io::Result<_>>()?;
        Ok(Self { path, policies })
    }

    fn save(&self) -> io::Result<()> {
        let mut text = String::new();
        for policy in self.policies.values() {
            let expiry = policy
                .expires_at
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?;
            writeln!(
                text,
                "{}\t{}\t{}\t{}",
                policy.host,
                policy.port,
                expiry.as_secs(),
                policy.duration.as_secs()
            )
            .map_err(io::Error::other)?;
        }
        fs::write(&self.path, text)
    }
}

impl StsStore for FileStore {
    fn get(&self, host: &str) -> Option<StsPolicy> {
        self.policies.get(host).cloned()
    }

    fn put(&mut self, policy: StsPolicy) {
        self.policies.insert(policy.host.clone(), policy);
        self.save().expect("persist STS policies");
    }

    fn remove(&mut self, host: &str) {
        self.policies.remove(host);
        self.save().expect("persist STS policies");
    }
}

fn parse_policy(line: &str) -> io::Result<(String, StsPolicy)> {
    let mut fields = line.split('\t');
    let (Some(host), Some(port), Some(expiry), Some(duration), None) = (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    ) else {
        return Err(io::Error::new(ErrorKind::InvalidData, "invalid STS policy"));
    };
    let number = |text: &str| text.parse::<u64>().map_err(io::Error::other);
    let policy = StsPolicy::new(
        host,
        u16::try_from(number(port)?).map_err(io::Error::other)?,
        UNIX_EPOCH + Duration::from_secs(number(expiry)?),
        Duration::from_secs(number(duration)?),
    );
    Ok((host.to_owned(), policy))
}

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args_os().nth(1).ok_or("usage: sts-store PATH")?;
    let mut store = FileStore::open(Path::new(&path))?;
    store.put(StsPolicy::new(
        "irc.example.net",
        6697,
        UNIX_EPOCH + Duration::from_secs(4_000_000_000),
        Duration::from_hours(4_320),
    ));
    store.put(StsPolicy::new(
        "irc.libera.chat",
        6697,
        UNIX_EPOCH + Duration::from_secs(4_100_000_000),
        Duration::from_hours(4_320),
    ));
    store.remove("irc.libera.chat");
    let reopened = FileStore::open(path)?;
    let policy = reopened
        .get("irc.example.net")
        .ok_or("policy was not saved")?;
    println!("{} requires TLS on port {}", policy.host, policy.port);
    if reopened.get("irc.libera.chat").is_some() {
        return Err("policy was not removed".into());
    }
    println!("irc.libera.chat policy removed");
    Ok(())
}
