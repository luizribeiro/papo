//! Persist `IRCv3` STS policies in an application-owned file.

use std::{
    collections::HashMap, env, error::Error, fmt::Write as _, fs, io, path::PathBuf,
    time::UNIX_EPOCH,
};

use papo::sts::{StsPolicy, StsStore};

struct FileStore {
    path: PathBuf,
    policies: HashMap<String, StsPolicy>,
}

impl FileStore {
    fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let text = fs::read_to_string(&path).or_else(|error| {
            (error.kind() == io::ErrorKind::NotFound)
                .then(String::new)
                .ok_or(error)
        })?;
        let policies = text.lines().map(parse_policy).collect::<io::Result<_>>()?;
        Ok(Self { path, policies })
    }

    fn save(&self) {
        let mut text = String::new();
        for policy in self.policies.values() {
            let expiry = policy.expires_at.duration_since(UNIX_EPOCH).unwrap();
            writeln!(
                text,
                "{}\t{}\t{}\t{}",
                policy.host,
                policy.port,
                expiry.as_secs(),
                policy.duration.as_secs()
            )
            .unwrap();
        }
        fs::write(&self.path, text).expect("persist STS policies");
    }
}

impl StsStore for FileStore {
    fn get(&self, host: &str) -> Option<StsPolicy> {
        self.policies.get(host).cloned()
    }

    fn put(&mut self, policy: StsPolicy) {
        self.policies.insert(policy.host.clone(), policy);
        self.save();
    }

    fn remove(&mut self, host: &str) {
        self.policies.remove(host);
        self.save();
    }
}

fn parse_policy(line: &str) -> io::Result<(String, StsPolicy)> {
    let fields = line.split('\t').collect::<Vec<_>>();
    let [host, port, expiry, duration] = fields.as_slice() else {
        return Err(io::Error::other("invalid STS policy"));
    };
    let policy = StsPolicy::new(
        *host,
        port.parse().map_err(io::Error::other)?,
        UNIX_EPOCH + std::time::Duration::from_secs(expiry.parse().map_err(io::Error::other)?),
        std::time::Duration::from_secs(duration.parse().map_err(io::Error::other)?),
    );
    Ok((host.to_string(), policy))
}

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args_os().nth(1).ok_or("usage: sts-store PATH")?;
    let mut store = FileStore::open(&path)?;
    store.put(StsPolicy::new(
        "irc.example.net",
        6697,
        UNIX_EPOCH + std::time::Duration::from_secs(4_000_000_000),
        std::time::Duration::from_hours(4_320),
    ));
    let policy = FileStore::open(path)?
        .get("irc.example.net")
        .ok_or("policy was not saved")?;
    println!("{} requires TLS on port {}", policy.host, policy.port);
    Ok(())
}
