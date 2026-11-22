use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};

const MAX_BYTES: u64 = 1024 * 1024;

fn directory() -> Result<PathBuf> {
    Ok(crate::session::get_app_dir_path()?.join("codex-launch-args"))
}

pub(super) fn write() -> Result<PathBuf> {
    let mut input = Vec::new();
    std::io::stdin()
        .take(MAX_BYTES + 1)
        .read_to_end(&mut input)?;
    ensure!(
        !input.is_empty() && input.len() as u64 <= MAX_BYTES && input.last() == Some(&0),
        "Invalid Codex argument stream"
    );
    let words = input[..input.len() - 1]
        .split(|byte| *byte == 0)
        .map(|word| String::from_utf8(word.to_vec()).context("Codex arguments are not UTF-8"))
        .collect::<Result<Vec<_>>>()?;
    let directory = directory()?;
    std::fs::create_dir_all(&directory)?;
    let metadata = std::fs::symlink_metadata(&directory)?;
    ensure!(
        metadata.file_type().is_dir() && metadata.uid() == crate::process::current_user_id(),
        "Codex argument directory must be owned and not a symlink"
    );
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    let mut file = tempfile::Builder::new()
        .prefix("launch.")
        .rand_bytes(16)
        .tempfile_in(&directory)?;
    let encoded = serde_json::to_vec(&words)?;
    ensure!(
        encoded.len() as u64 <= MAX_BYTES,
        "Codex argument JSON exceeds size limit"
    );
    file.write_all(&encoded)?;
    file.as_file().sync_all()?;
    Ok(file.keep().context("Keeping private Codex arguments")?.1)
}

pub(super) fn read(path: &Path) -> Result<Vec<String>> {
    validate_path(path)?;
    ensure!(
        std::fs::symlink_metadata(path)?.file_type().is_file(),
        "Codex argument path is not a regular file"
    );
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.len() <= MAX_BYTES
            && metadata.uid() == crate::process::current_user_id()
            && metadata.permissions().mode() & 0o077 == 0,
        "Codex argument file is not private"
    );
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "Codex argument file exceeds size limit"
    );
    let words = serde_json::from_slice(&bytes).map_err(|error| {
        anyhow::anyhow!(
            "Invalid Codex argument schema at line {}, column {}",
            error.line(),
            error.column()
        )
    })?;
    std::fs::remove_file(path).context("Removing consumed Codex argument file")?;
    Ok(words)
}

fn validate_path(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(directory()?)?;
    ensure!(
        metadata.file_type().is_dir()
            && metadata.permissions().mode() & 0o077 == 0
            && metadata.uid() == crate::process::current_user_id(),
        "Codex argument directory is not private or has another owner"
    );
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("Invalid Codex argument filename")?;
    let suffix = name
        .strip_prefix("launch.")
        .context("Invalid Codex argument file")?;
    ensure!(
        (8..=64).contains(&suffix.len()) && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric()),
        "Invalid Codex argument file token"
    );
    ensure!(
        path.parent()
            .context("Missing Codex argument directory")?
            .canonicalize()?
            == directory()?.canonicalize()?,
        "Codex argument file is outside its private directory"
    );
    Ok(())
}
