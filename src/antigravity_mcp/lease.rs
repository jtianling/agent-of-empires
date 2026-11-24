use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::ErrorKind;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{bail, ensure, Context, Result};
use sha2::{Digest, Sha256};

const ACQUIRE_WAIT: Duration = Duration::from_secs(3);
const ACQUIRE_POLL: Duration = Duration::from_millis(50);

pub(super) fn acquire(app_dir: &Path, key: &str) -> Result<File> {
    acquire_within(app_dir, key, ACQUIRE_WAIT)
}

pub(super) fn acquire_within(app_dir: &Path, key: &str, timeout: Duration) -> Result<File> {
    let dir = app_dir.join("antigravity-mcp-locks");
    match DirBuilder::new().mode(0o700).create(&dir) {
        Err(err) if err.kind() == ErrorKind::AlreadyExists => {}
        result => result.context("Creating Antigravity lock directory")?,
    }
    let uid = crate::process::current_user_id();
    let meta = fs::symlink_metadata(&dir)?;
    ensure!(
        meta.is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0,
        "Antigravity lock directory is not private"
    );
    let digest = Sha256::digest(key.as_bytes());
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(dir.join(format!("{digest:x}")))
        .context("Opening Antigravity pane lock")?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.uid() == uid && meta.mode() & 0o077 == 0,
        "Antigravity lock file is not private"
    );
    // Keep the inode in place across owners. Unlinking creates a second lock domain.
    // Wait briefly so an in-place respawn can finish tearing down the previous bridge.
    let deadline = Instant::now() + timeout;
    loop {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(file),
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    bail!("Antigravity pane bridge already active");
                }
                std::thread::sleep(remaining.min(ACQUIRE_POLL));
            }
            Err(err) => return Err(err).context("Locking Antigravity pane"),
        }
    }
}
