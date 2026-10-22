use std::{collections::HashMap, fmt::Write as _, fs, io, path::PathBuf, time::UNIX_EPOCH};

use papo::sts::{StsPolicy, StsStore};

pub(crate) struct FileStore {
    path: PathBuf,
    policies: HashMap<String, StsPolicy>,
}

impl FileStore {
    pub(crate) fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
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
            let (host, port, expiry, duration, transport) = (
                &policy.host,
                policy.port,
                expiry.as_secs(),
                policy.duration.as_secs(),
                if policy.websocket { "websocket" } else { "tcp" },
            );
            writeln!(text, "{host}\t{port}\t{expiry}\t{duration}\t{transport}").unwrap();
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
    let (host, port, expiry, duration, transport) = match fields.as_slice() {
        [host, port, expiry, duration] => (*host, *port, *expiry, *duration, false),
        [host, port, expiry, duration, transport] => {
            let transport = match *transport {
                "tcp" => false,
                "websocket" => true,
                _ => return Err(io::Error::other("invalid STS policy transport")),
            };
            (*host, *port, *expiry, *duration, transport)
        }
        _ => return Err(io::Error::other("invalid STS policy")),
    };
    let policy = StsPolicy::new(
        host,
        port.parse().map_err(io::Error::other)?,
        UNIX_EPOCH + std::time::Duration::from_secs(expiry.parse().map_err(io::Error::other)?),
        std::time::Duration::from_secs(duration.parse().map_err(io::Error::other)?),
    )
    .websocket(transport);
    Ok((host.to_string(), policy))
}
