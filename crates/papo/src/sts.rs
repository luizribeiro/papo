//! Strict Transport Security policy stores.

pub use papo_core::sts::{MemoryStsStore, StoredPolicy, StsStore};

#[cfg(feature = "sts-file")]
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

#[cfg(feature = "sts-file")]
use serde::{Deserialize, Serialize};

/// A write-through JSON file store for STS policies.
#[cfg(feature = "sts-file")]
#[derive(Debug)]
pub struct FileStsStore {
    path: PathBuf,
    entries: BTreeMap<String, DiskPolicy>,
    last_error: Option<io::Error>,
}

#[cfg(feature = "sts-file")]
impl FileStsStore {
    /// Opens `path`, loading its policies once.
    #[must_use]
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut store = Self {
            path,
            entries: BTreeMap::new(),
            last_error: None,
        };
        match fs::read(&store.path) {
            Ok(bytes) => match serde_json::from_slice::<BTreeMap<String, DiskPolicy>>(&bytes) {
                Ok(entries) => store.load(entries),
                Err(error) => store.last_error = Some(invalid_data(error)),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => store.last_error = Some(error),
        }
        store
    }

    /// Returns the most recent load or write error.
    #[must_use]
    pub fn last_error(&self) -> Option<&io::Error> {
        self.last_error.as_ref()
    }

    fn load(&mut self, entries: BTreeMap<String, DiskPolicy>) {
        for (host, entry) in entries {
            self.entries.insert(normalize_host(&host), entry);
        }
    }

    fn persist(&mut self) {
        match write_atomic(&self.path, &self.entries) {
            Ok(()) => self.last_error = None,
            Err(error) => self.last_error = Some(error),
        }
    }
}

#[cfg(feature = "sts-file")]
impl StsStore for FileStsStore {
    fn get(&self, host: &str) -> Option<StoredPolicy> {
        let host = normalize_host(host);
        self.entries
            .get(&host)
            .and_then(|entry| entry.to_stored(host))
    }

    fn put(&mut self, policy: StoredPolicy) {
        let host = normalize_host(&policy.host);
        self.entries.insert(host, DiskPolicy::from_stored(&policy));
        self.persist();
    }

    fn remove(&mut self, host: &str) {
        self.entries.remove(&normalize_host(host));
        self.persist();
    }
}

#[cfg(feature = "sts-file")]
#[derive(Clone, Debug, Deserialize, Serialize)]
struct DiskPolicy {
    #[serde(default, rename = "port", skip_serializing_if = "Option::is_none")]
    _port: Option<u16>,
    duration_secs: u64,
    expires_at_unix: u64,
}

#[cfg(feature = "sts-file")]
impl DiskPolicy {
    fn from_stored(policy: &StoredPolicy) -> Self {
        Self {
            _port: None,
            duration_secs: policy.duration.as_secs(),
            expires_at_unix: policy
                .expires_at
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }

    fn to_stored(&self, host: String) -> Option<StoredPolicy> {
        Some(StoredPolicy {
            host,
            expires_at: UNIX_EPOCH.checked_add(Duration::from_secs(self.expires_at_unix))?,
            duration: Duration::from_secs(self.duration_secs),
        })
    }
}

#[cfg(feature = "sts-file")]
fn write_atomic(path: &Path, entries: &BTreeMap<String, DiskPolicy>) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec(entries).map_err(invalid_data)?;
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    let result = fs::write(&temp, bytes).and_then(|()| fs::rename(&temp, path));
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(feature = "sts-file")]
fn invalid_data(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

#[cfg(feature = "sts-file")]
fn normalize_host(host: &str) -> String {
    let mut store = MemoryStsStore::new();
    store.put(StoredPolicy {
        host: host.to_owned(),
        expires_at: UNIX_EPOCH,
        duration: Duration::ZERO,
    });
    store
        .get(host)
        .map_or_else(|| host.to_owned(), |policy| policy.host)
}

#[cfg(all(test, feature = "sts-file"))]
mod tests {
    use std::{fs, path::PathBuf, time::Duration};

    use super::{FileStsStore, StoredPolicy, StsStore};

    fn test_dir(name: &str) -> PathBuf {
        let path = PathBuf::from("target/sts-file-tests").join(name);
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn policy(host: &str, expires_at_unix: u64) -> StoredPolicy {
        StoredPolicy {
            host: host.to_owned(),
            expires_at: std::time::UNIX_EPOCH + Duration::from_secs(expires_at_unix),
            duration: Duration::from_secs(15_552_000),
        }
    }

    #[test]
    fn round_trips_across_reopening() {
        let path = test_dir("round-trip").join("sts.json");
        let mut store = FileStsStore::open(&path);
        store.put(policy("irc.example.net", 4_000_000_000));
        store.put(policy("irc.libera.chat", 4_100_000_000));
        store.remove("irc.libera.chat");
        assert!(store.last_error().is_none());

        let reopened = FileStsStore::open(path);
        assert_eq!(
            reopened.get("irc.example.net"),
            Some(policy("irc.example.net", 4_000_000_000))
        );
        assert!(reopened.get("irc.libera.chat").is_none());
    }

    // WASI hosts do not reliably enforce directory mode bits for preopened directories.
    #[cfg(all(unix, not(target_os = "wasi")))]
    #[test]
    fn a_failed_write_leaves_the_previous_file_intact() {
        use std::os::unix::fs::PermissionsExt;

        let dir = test_dir("atomic-failure");
        let path = dir.join("sts.json");
        let mut store = FileStsStore::open(&path);
        store.put(policy("irc.example.net", 4_000_000_000));
        let original = fs::read(&path).unwrap();

        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        store.put(policy("irc.example.net", 4_100_000_000));
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(store.last_error().is_some());
        assert_eq!(
            store.get("irc.example.net"),
            Some(policy("irc.example.net", 4_100_000_000))
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(!PathBuf::from(format!("{}.tmp", path.display())).exists());
    }
}
