//! Strict Transport Security policy stores.

pub use papo_core::sts::{MemoryStsStore, StoredPolicy, StsStore};

#[cfg(feature = "sts-file")]
use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(feature = "sts-file")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "sts-file")]
const MAX_FILE_BYTES: u64 = 1024 * 1024;
#[cfg(feature = "sts-file")]
const MAX_ENTRIES: usize = 1024;

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
        match read_entries(&store.path) {
            Ok(Some(entries)) => {
                if let Err(error) = store.load(entries, SystemTime::now()) {
                    store.last_error = Some(error);
                }
            }
            Ok(None) => {}
            Err(error) => store.last_error = Some(error),
        }
        store
    }

    /// Returns the platform's default STS policy file, when its home is known.
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        default_path_with(|name| env::var_os(name), cfg!(target_os = "macos"))
    }

    /// Returns the most recent load or write error.
    #[must_use]
    pub fn last_error(&self) -> Option<&io::Error> {
        self.last_error.as_ref()
    }

    fn load(&mut self, entries: BTreeMap<String, DiskPolicy>, now: SystemTime) -> io::Result<()> {
        let mut loaded = BTreeMap::new();
        for (host, entry) in entries {
            let host = normalize_host(&host);
            let policy = entry
                .to_stored(host.clone())
                .ok_or_else(|| invalid_data("STS expiry is outside the system clock"))?;
            if policy.is_active_at(now) {
                loaded.insert(host, entry);
            }
        }
        cap_entries(&mut loaded);
        self.entries = loaded;
        Ok(())
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
        cap_entries(&mut self.entries);
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
fn read_entries(path: &Path) -> io::Result<Option<BTreeMap<String, DiskPolicy>>> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if metadata.len() > MAX_FILE_BYTES {
        return Err(invalid_data("STS file exceeds 1048576 bytes"));
    }
    let bytes = fs::read(path)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(invalid_data("STS file exceeds 1048576 bytes"));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(invalid_data)
}

#[cfg(feature = "sts-file")]
fn cap_entries(entries: &mut BTreeMap<String, DiskPolicy>) {
    let excess = entries.len().saturating_sub(MAX_ENTRIES);
    let mut expiry_order: Vec<_> = entries
        .iter()
        .map(|(host, entry)| (entry.expires_at_unix, host.clone()))
        .collect();
    expiry_order.sort_unstable();
    for (_, host) in expiry_order.into_iter().take(excess) {
        entries.remove(&host);
    }
}

#[cfg(feature = "sts-file")]
fn default_path_with(
    mut lookup: impl FnMut(&str) -> Option<OsString>,
    is_macos: bool,
) -> Option<PathBuf> {
    if is_macos {
        return nonempty(lookup("HOME"))
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support/papo/sts.json"));
    }
    if let Some(state) = nonempty(lookup("XDG_STATE_HOME")) {
        return Some(PathBuf::from(state).join("papo/sts.json"));
    }
    nonempty(lookup("HOME"))
        .map(PathBuf::from)
        .map(|home| home.join(".local/state/papo/sts.json"))
}

#[cfg(feature = "sts-file")]
fn nonempty(value: Option<OsString>) -> Option<OsString> {
    value.filter(|value| !value.is_empty())
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
    use std::{fs, io, path::PathBuf, time::Duration};

    use super::{FileStsStore, StoredPolicy, StsStore, default_path_with};

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

    #[test]
    fn creates_missing_parent_directories() {
        let path = test_dir("missing-parent").join("nested/papo/sts.json");
        let mut store = FileStsStore::open(&path);
        store.put(policy("irc.example.net", 4_000_000_000));

        assert!(store.last_error().is_none());
        assert_eq!(
            FileStsStore::open(path).get("irc.example.net"),
            Some(policy("irc.example.net", 4_000_000_000))
        );
    }

    #[test]
    fn a_missing_file_loads_empty_without_an_error() {
        let path = test_dir("missing-file").join("sts.json");
        let store = FileStsStore::open(path);

        assert!(store.last_error().is_none());
        assert!(store.get("irc.example.net").is_none());
    }

    #[test]
    fn a_directory_at_the_file_path_loads_empty_with_an_error() {
        let path = test_dir("directory-path").join("sts.json");
        fs::create_dir(&path).unwrap();
        let store = FileStsStore::open(path);

        assert!(store.last_error().is_some());
        assert!(store.get("irc.example.net").is_none());
    }

    // WASI hosts do not reliably enforce directory mode bits for preopened directories.
    #[cfg(all(unix, not(target_os = "wasi")))]
    #[test]
    fn a_file_in_place_of_the_parent_directory_loads_empty_with_an_error() {
        let parent = test_dir("parent-file").join("not-a-directory");
        fs::write(&parent, b"").unwrap();
        let store = FileStsStore::open(parent.join("sts.json"));
        assert!(store.last_error().is_some());
        assert!(store.get("irc.example.net").is_none());
    }

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

    #[test]
    fn drops_expired_entries_on_load() {
        let path = test_dir("expired").join("sts.json");
        fs::write(
            &path,
            r#"{"irc.expired.net":{"duration_secs":60,"expires_at_unix":1},"irc.example.net":{"duration_secs":15552000,"expires_at_unix":4000000000}}"#,
        )
        .unwrap();

        let store = FileStsStore::open(path);
        assert!(store.get("irc.expired.net").is_none());
        assert_eq!(
            store.get("irc.example.net"),
            Some(policy("irc.example.net", 4_000_000_000))
        );
    }

    #[test]
    fn a_corrupt_file_loads_empty_and_reports_the_error() {
        let path = test_dir("corrupt").join("sts.json");
        fs::write(&path, b"{not valid JSON").unwrap();

        let store = FileStsStore::open(path);
        assert_eq!(
            store.last_error().map(io::Error::kind),
            Some(io::ErrorKind::InvalidData)
        );
        assert!(store.get("irc.example.net").is_none());
    }

    #[test]
    fn accepts_one_mebibyte_but_rejects_one_byte_more() {
        let path = test_dir("oversized").join("sts.json");
        let mut bytes = b"{}".to_vec();
        bytes.resize(1_048_576, b' ');
        fs::write(&path, &bytes).unwrap();
        assert!(FileStsStore::open(&path).last_error().is_none());

        bytes.push(b' ');
        fs::write(&path, bytes).unwrap();
        let store = FileStsStore::open(path);
        assert_eq!(
            store.last_error().map(io::Error::kind),
            Some(io::ErrorKind::InvalidData)
        );
        assert!(store.get("irc.example.net").is_none());
    }

    #[test]
    fn keeps_the_1024_latest_expiries() {
        let path = test_dir("entry-cap").join("sts.json");
        let mut entries = std::collections::BTreeMap::new();
        for index in 0..1025 {
            entries.insert(
                format!("irc-{index}.example.net"),
                super::DiskPolicy {
                    _port: None,
                    duration_secs: 60,
                    expires_at_unix: 4_000_000_000 + index,
                },
            );
        }
        fs::write(&path, serde_json::to_vec(&entries).unwrap()).unwrap();

        let mut store = FileStsStore::open(&path);
        assert!(store.get("irc-0.example.net").is_none());
        assert!(store.get("irc-1.example.net").is_some());
        assert!(store.get("irc-1024.example.net").is_some());
        store.put(policy("irc-new.example.net", 4_100_000_000));
        assert!(store.get("irc-1.example.net").is_none());
        assert!(
            FileStsStore::open(path)
                .get("irc-new.example.net")
                .is_some()
        );
    }

    #[test]
    fn normalizes_hostnames_with_the_core_store() {
        let path = test_dir("normalization").join("sts.json");
        let mut store = FileStsStore::open(&path);
        store.put(policy("IRC.Example.NET.", 4_000_000_000));

        assert_eq!(
            store.get("irc.example.net"),
            Some(policy("irc.example.net", 4_000_000_000))
        );
        let json = fs::read_to_string(&path).unwrap();
        assert!(json.contains(r#""irc.example.net""#));
        assert!(!json.contains("IRC.Example.NET."));
        assert!(FileStsStore::open(path).get("IRC.EXAMPLE.NET.").is_some());
    }

    #[test]
    fn locates_the_default_file_from_an_injected_environment() {
        let path = |variables: &[(&str, &str)], macos| {
            default_path_with(
                |name| {
                    variables
                        .iter()
                        .find_map(|(key, value)| (*key == name).then(|| (*value).into()))
                },
                macos,
            )
        };

        assert_eq!(
            path(
                &[("XDG_STATE_HOME", "/state"), ("HOME", "/home/irc")],
                false
            ),
            Some(PathBuf::from("/state/papo/sts.json"))
        );
        assert_eq!(
            path(&[("XDG_STATE_HOME", "/state")], false),
            Some(PathBuf::from("/state/papo/sts.json"))
        );
        assert_eq!(
            path(&[("HOME", "/home/irc")], false),
            Some(PathBuf::from("/home/irc/.local/state/papo/sts.json"))
        );
        assert_eq!(
            path(
                &[("XDG_STATE_HOME", "/state"), ("HOME", "/Users/irc")],
                true
            ),
            Some(PathBuf::from(
                "/Users/irc/Library/Application Support/papo/sts.json"
            ))
        );
        assert_eq!(path(&[("XDG_STATE_HOME", "/state")], true), None);
        assert_eq!(path(&[("XDG_STATE_HOME", ""), ("HOME", "")], false), None);
        assert_eq!(path(&[], false), None);
    }
}
