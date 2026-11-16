use std::{
    collections::HashMap,
    fmt::Write as _,
    fs, io,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, UNIX_EPOCH},
};

use papo::sts::{StsPolicy, StsStore};

pub(crate) struct FileStore {
    inner: Mutex<FileStoreInner>,
}

struct FileStoreInner {
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
        Ok(Self {
            inner: Mutex::new(FileStoreInner { path, policies }),
        })
    }

    fn save(inner: &FileStoreInner) {
        let mut text = String::new();
        for policy in inner.policies.values() {
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
        fs::write(&inner.path, text).expect("persist STS policies");
    }
}

impl StsStore for FileStore {
    fn get(&self, host: &str) -> Option<StsPolicy> {
        self.inner.lock().unwrap().policies.get(host).cloned()
    }

    fn put(&self, policy: StsPolicy) {
        let mut inner = self.inner.lock().unwrap();
        inner.policies.insert(policy.host.clone(), policy);
        Self::save(&inner);
    }

    fn remove(&self, host: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.policies.remove(host);
        Self::save(&inner);
    }
}

fn parse_policy(line: &str) -> io::Result<(String, StsPolicy)> {
    let [host, port, expiry, duration, transport] = line.split('\t').collect::<Vec<_>>()[..] else {
        return Err(io::Error::other("invalid STS policy"));
    };
    let websocket = match transport {
        "tcp" => false,
        "websocket" => true,
        _ => return Err(io::Error::other("invalid STS policy transport")),
    };
    let policy = StsPolicy::new(
        host,
        port.parse().map_err(io::Error::other)?,
        UNIX_EPOCH + Duration::from_secs(expiry.parse().map_err(io::Error::other)?),
        Duration::from_secs(duration.parse().map_err(io::Error::other)?),
    )
    .websocket(websocket);
    Ok((host.to_string(), policy))
}
