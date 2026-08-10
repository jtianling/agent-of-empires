//! Discovery of the user-owned kimi server.
//!
//! The server is a shared singleton that also serves kimi sessions AoE never
//! launched, so this module only ever reads: it finds a live instance, reports
//! whether that instance can host a pane's session, and never starts, restarts
//! or terminates anything.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// Cap on a single registry entry. The registry is written by peers that may be
/// mid-write, so a file larger than a small record is not a record.
const MAX_ENTRY_BYTES: u64 = 4 * 1024;
const MAX_TOKEN_BYTES: u64 = 4 * 1024;
const KIMI_HOME_ENV: &str = "KIMI_CODE_HOME";
#[cfg(test)]
pub(crate) const KIMI_HOME_ENV_FOR_TESTS: &str = KIMI_HOME_ENV;

/// One entry of `<kimi home>/server/instances/<server_id>.json`.
///
/// Decoded leniently on purpose: the registry is a public contract owned by the
/// kimi server, and a field added there must not make AoE stop seeing a live
/// instance. `heartbeat_at` and `host_version` are read by nothing here --
/// heartbeat only says a file is being refreshed, and a dev build reports the
/// upstream version it was branched from.
#[derive(Debug, Deserialize)]
struct InstanceEntry {
    server_id: String,
    pid: u32,
    host: String,
    port: u16,
    started_at: i64,
}

/// A live kimi server instance AoE may connect to but must never manage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KimiServer {
    pub server_id: String,
    pub base_url: String,
    pub port: u16,
    pub pid: u32,
    home: PathBuf,
}

/// `$KIMI_CODE_HOME`, or `~/.kimi-code`.
pub fn kimi_home() -> Result<PathBuf> {
    match std::env::var_os(KIMI_HOME_ENV) {
        Some(value) if !value.is_empty() => Ok(PathBuf::from(value)),
        Some(_) => bail!("{KIMI_HOME_ENV} must not be empty"),
        None => Ok(dirs::home_dir()
            .context("finding home directory for kimi server discovery")?
            .join(".kimi-code")),
    }
}

/// Select the earliest started live instance, the way every other single
/// instance consumer of this registry does, so AoE and the kimi CLI land on the
/// same server.
pub fn discover() -> Result<KimiServer> {
    discover_in(&kimi_home()?)
}

pub(crate) fn discover_in(home: &Path) -> Result<KimiServer> {
    let directory = instances_dir(home);
    let mut entries = read_instances(&directory);
    entries.sort_by_key(|entry| entry.started_at);
    let live = entries.into_iter().find(|entry| pid_is_live(entry.pid));
    let entry = live.with_context(|| {
        format!(
            "no live kimi server found in '{}'. Start the shared kimi server \
             (for example `kimi web --no-open`) and relaunch this pane; AoE \
             never starts or stops it.",
            directory.display()
        )
    })?;
    Ok(KimiServer {
        base_url: format!("http://{}:{}", entry.host, entry.port),
        server_id: entry.server_id,
        port: entry.port,
        pid: entry.pid,
        home: home.to_path_buf(),
    })
}

fn instances_dir(home: &Path) -> PathBuf {
    home.join("server").join("instances")
}

/// Every readable, well formed entry of the registry directory.
///
/// An entry that cannot be read or decoded is skipped and left in place: it may
/// be a peer's half-written file, and deleting another process's record is not
/// AoE's to do. Only `.json` is considered, which also skips the temporary
/// names an atomic write leaves behind.
fn read_instances(directory: &Path) -> Vec<InstanceEntry> {
    let Ok(listing) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    listing
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| read_instance(&entry.path()))
        .collect()
}

fn read_instance(path: &Path) -> Option<InstanceEntry> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_ENTRY_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let entry: InstanceEntry = serde_json::from_slice(&bytes).ok()?;
    let loopback = matches!(entry.host.as_str(), "127.0.0.1" | "localhost" | "::1");
    if entry.server_id.is_empty() || entry.pid <= 1 || entry.port == 0 || !loopback {
        return None;
    }
    Some(entry)
}

/// Liveness is process existence and nothing else.
///
/// A conservative reading in both unusual directions: `EPERM` means the process
/// exists under another uid, and any other errno leaves the instance alive
/// rather than letting an unexpected failure look like a dead server.
fn pid_is_live(pid: u32) -> bool {
    if pid > i32::MAX as u32 {
        return false;
    }
    !matches!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None),
        Err(nix::errno::Errno::ESRCH)
    )
}

impl KimiServer {
    /// The klient IPC socket this instance advertises, if it mounted one.
    fn ipc_socket(&self) -> PathBuf {
        self.home
            .join("server")
            .join(format!("klient-{}.sock", self.port))
    }

    /// Whether this instance can host a pane's session for a TUI attached to it.
    ///
    /// The only positive signal is the klient IPC socket: an engine that cannot
    /// serve a TUI never mounts it. A version number cannot stand in, because a
    /// branch build reports the upstream release it was branched from.
    ///
    /// A crash leaves the socket file behind, so its presence only counts while
    /// the instance that advertised it is still running.
    pub fn has_capable_engine(&self) -> bool {
        pid_is_live(self.pid) && self.ipc_socket().exists()
    }

    /// Refuse the launch unless the server can host the pane's session.
    pub fn require_capable_engine(&self) -> Result<()> {
        if self.has_capable_engine() {
            return Ok(());
        }
        bail!(
            "kimi server {} on port {} does not expose the klient IPC socket at \
             '{}', so a Cross Agent Team pane would run its own engine and every \
             poked turn would land in a session you cannot see. Restart the \
             shared server with a build that mounts that socket.",
            self.server_id,
            self.port,
            self.ipc_socket().display()
        )
    }

    /// The shared bearer token every REST call to this server needs.
    pub fn read_token(&self) -> Result<String> {
        read_token_in(&self.home)
    }
}

pub(crate) fn read_token_in(home: &Path) -> Result<String> {
    let path = home.join("server.token");
    let metadata = std::fs::metadata(&path)
        .with_context(|| format!("reading kimi server token '{}'", path.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_TOKEN_BYTES {
        bail!("kimi server token '{}' is invalid", path.display());
    }
    let token = std::fs::read_to_string(&path)
        .with_context(|| format!("reading kimi server token '{}'", path.display()))?;
    let token = token.trim().to_string();
    if token.is_empty() || token.bytes().any(|byte| matches!(byte, b'\r' | b'\n')) {
        bail!(
            "kimi server token '{}' is empty or malformed",
            path.display()
        );
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_entry(home: &Path, name: &str, body: &str) {
        let directory = instances_dir(home);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(name), body).unwrap();
    }

    fn entry_json(server_id: &str, pid: u32, port: u16, started_at: i64) -> String {
        format!(
            r#"{{"server_id":"{server_id}","pid":{pid},"host":"127.0.0.1",
                 "port":{port},"started_at":{started_at},"heartbeat_at":1,
                 "host_version":"0.34.0"}}"#
        )
    }

    #[test]
    fn empty_or_missing_registry_fails_closed_with_the_directory() {
        let home = tempfile::tempdir().unwrap();
        let error = discover_in(home.path()).unwrap_err();
        let diagnostic = format!("{error:#}");
        assert!(diagnostic.contains("no live kimi server"));
        assert!(diagnostic.contains("instances"));

        std::fs::create_dir_all(instances_dir(home.path())).unwrap();
        assert!(discover_in(home.path()).is_err());
    }

    #[test]
    fn earliest_started_live_instance_wins_over_later_and_dead_ones() {
        let home = tempfile::tempdir().unwrap();
        let live = std::process::id();
        let dead = i32::MAX as u32;
        write_entry(home.path(), "dead.json", &entry_json("dead", dead, 1111, 1));
        write_entry(
            home.path(),
            "late.json",
            &entry_json("late", live, 3333, 30),
        );
        write_entry(
            home.path(),
            "early.json",
            &entry_json("early", live, 2222, 20),
        );
        let server = discover_in(home.path()).unwrap();
        assert_eq!(server.server_id, "early");
        assert_eq!(server.base_url, "http://127.0.0.1:2222");
    }

    #[test]
    fn unreadable_entries_are_ignored_for_selection_and_left_on_disk() {
        let home = tempfile::tempdir().unwrap();
        let live = std::process::id();
        write_entry(home.path(), "broken.json", "{not json");
        write_entry(home.path(), "huge.json", &"x".repeat(5 * 1024));
        write_entry(home.path(), "partial.json", r#"{"pid":123}"#);
        // A temp file an atomic write is about to rename into place.
        write_entry(
            home.path(),
            "pending.json.tmp",
            &entry_json("tmp", live, 1, 1),
        );
        write_entry(
            home.path(),
            "good.json",
            &entry_json("good", live, 4444, 50),
        );

        let server = discover_in(home.path()).unwrap();
        assert_eq!(server.server_id, "good");
        for name in ["broken.json", "huge.json", "partial.json"] {
            assert!(instances_dir(home.path()).join(name).exists());
        }
    }

    #[test]
    fn a_stale_heartbeat_alone_never_marks_an_instance_dead() {
        let home = tempfile::tempdir().unwrap();
        let stale = format!(
            r#"{{"server_id":"stale-beat","pid":{},"host":"127.0.0.1","port":5555,
                 "started_at":10,"heartbeat_at":0}}"#,
            std::process::id()
        );
        write_entry(home.path(), "stale.json", &stale);
        assert_eq!(discover_in(home.path()).unwrap().server_id, "stale-beat");
    }

    #[test]
    fn capability_needs_the_socket_of_a_still_running_instance() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("server")).unwrap();
        write_entry(
            home.path(),
            "one.json",
            &entry_json("one", std::process::id(), 6060, 10),
        );
        let server = discover_in(home.path()).unwrap();

        assert!(!server.has_capable_engine());
        let error = server.require_capable_engine().unwrap_err();
        assert!(format!("{error:#}").contains("klient-6060.sock"));

        std::fs::write(home.path().join("server").join("klient-6060.sock"), "").unwrap();
        assert!(server.has_capable_engine());

        // A socket left behind by a crashed instance is not a signal.
        let orphan = KimiServer {
            pid: i32::MAX as u32,
            ..server.clone()
        };
        assert!(!orphan.has_capable_engine());
    }

    #[test]
    fn token_is_read_trimmed_and_rejected_when_absent_or_empty() {
        let home = tempfile::tempdir().unwrap();
        assert!(read_token_in(home.path()).is_err());
        std::fs::write(home.path().join("server.token"), "  sk-abc123\n").unwrap();
        assert_eq!(read_token_in(home.path()).unwrap(), "sk-abc123");
        std::fs::write(home.path().join("server.token"), "   \n").unwrap();
        assert!(read_token_in(home.path()).is_err());
    }
}
