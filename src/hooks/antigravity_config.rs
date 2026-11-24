use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::Path;

use anyhow::{ensure, Context, Result};
use serde_json::{Map, Value};

const MAX_BYTES: u64 = 4 * 1024 * 1024;

pub(super) fn update(
    path: &Path,
    create: bool,
    edit: impl FnOnce(&mut Map<String, Value>) -> Result<bool>,
) -> Result<()> {
    let target = match path.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if !create {
                return Ok(());
            }
            ensure!(!path.is_symlink(), "Broken Antigravity config symlink");
            path.to_path_buf()
        }
        Err(error) => return Err(error.into()),
    };
    let dir = target.parent().context("Missing config directory")?;
    fs::create_dir_all(dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(".aoe-antigravity.lock"))?;
    fs2::FileExt::lock_exclusive(&lock)?;
    let original = read(&target)?;
    let mut object = match original.as_deref() {
        Some(bytes) => serde_json::from_slice(bytes)
            .with_context(|| format!("Invalid Antigravity JSON in {}", target.display()))?,
        None => Map::new(),
    };
    if !edit(&mut object)? {
        return Ok(());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer_pretty(&mut temporary, &object)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    ensure!(
        read(&target)? == original,
        "Antigravity config changed during update"
    );
    temporary.persist(&target)?;
    Ok(())
}

pub(super) fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(file.metadata()?.is_file(), "Expected a regular config file");
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "Antigravity config exceeds 4 MiB"
    );
    Ok(Some(bytes))
}
