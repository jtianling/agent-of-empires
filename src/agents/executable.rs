use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub(crate) fn resolve(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let nvm_dir = std::env::var_os("NVM_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".nvm")));
    resolve_in(binary, &path, nvm_dir.as_deref())
}

fn resolve_in(binary: &str, path: &OsStr, nvm_dir: Option<&Path>) -> Option<PathBuf> {
    if Path::new(binary).components().count() != 1 {
        return None;
    }
    if let Some(found) = std::env::split_paths(path)
        .filter_map(absolute_directory)
        .map(|dir| dir.join(binary))
        .find(|candidate| is_executable(candidate))
    {
        return Some(found);
    }
    let versions_dir = absolute_directory(nvm_dir?.to_path_buf())?.join("versions/node");
    let entries = match std::fs::read_dir(&versions_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!("Cannot search {}: {}", versions_dir.display(), error);
            return None;
        }
    };
    let mut versions = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => {
                if let Some(version) = node_version(&entry.file_name().to_string_lossy()) {
                    versions.push((version, entry.path()));
                }
            }
            Err(error) => {
                tracing::warn!("Cannot read entry in {}: {}", versions_dir.display(), error);
            }
        }
    }
    versions.sort_by(|left, right| right.0.cmp(&left.0));
    versions
        .into_iter()
        .map(|(_, dir)| dir.join("bin").join(binary))
        .find(|candidate| is_executable(candidate))
}

fn node_version(name: &str) -> Option<[u32; 3]> {
    let parts: Vec<_> = name.strip_prefix('v')?.split('.').collect();
    let [major, minor, patch] = parts.as_slice() else {
        return None;
    };
    Some([
        major.parse().ok()?,
        minor.parse().ok()?,
        patch.parse().ok()?,
    ])
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;

    match path.metadata() {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return false,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Err(error) => {
            tracing::warn!("Cannot inspect executable {}: {}", path.display(), error);
            return false;
        }
    }
    let path = match std::ffi::CString::new(path.as_os_str().as_bytes()) {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!("Invalid executable path {}: {}", path.display(), error);
            return false;
        }
    };
    // SAFETY: path is a valid, NUL-terminated C string for the duration of access.
    unsafe { nix::libc::access(path.as_ptr(), nix::libc::X_OK) == 0 }
}

fn absolute_directory(dir: PathBuf) -> Option<PathBuf> {
    if dir.is_absolute() {
        return Some(dir);
    }
    match std::env::current_dir() {
        Ok(cwd) => Some(cwd.join(dir)),
        Err(error) => {
            tracing::warn!("Cannot resolve relative executable directory: {}", error);
            None
        }
    }
}

pub(crate) fn fallback_setup(binary: &str) -> String {
    let Some(path) = resolve(binary) else {
        return String::new();
    };
    setup_for(binary, &path)
}

fn setup_for(binary: &str, path: &Path) -> String {
    let Some(dir) = path.parent().and_then(|path| path.to_str()) else {
        return String::new();
    };
    let binary = crate::session::environment::shell_escape(binary);
    let dir = crate::session::environment::shell_escape(dir);
    format!(
        "if ! command -v {binary} >/dev/null 2>&1; then \
         export PATH=\"${{PATH:-}}\":{dir}; fi; "
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn executable(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("codex");
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn path_takes_precedence_over_nvm() {
        let temp = tempfile::tempdir().unwrap();
        let expected = executable(&temp.path().join("bin"));
        executable(&temp.path().join("versions/node/v25.1.0/bin"));
        assert_eq!(
            resolve_in(
                "codex",
                temp.path().join("bin").as_os_str(),
                Some(temp.path())
            ),
            Some(expected)
        );
    }

    #[test]
    fn relative_path_directories_are_resolved_from_current_directory() {
        let cwd = std::env::current_dir().unwrap();
        let temp = tempfile::tempdir_in(&cwd).unwrap();
        let expected = executable(temp.path());
        let relative = temp.path().strip_prefix(&cwd).unwrap();
        assert_eq!(
            resolve_in("codex", relative.as_os_str(), None),
            Some(expected)
        );
        assert_eq!(absolute_directory(PathBuf::new()), Some(cwd.join("")));
    }

    #[test]
    fn execute_permission_is_checked_for_current_user() {
        let temp = tempfile::tempdir().unwrap();
        let path = executable(temp.path());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o001)).unwrap();
        let expected = std::process::Command::new("/bin/test")
            .arg("-x")
            .arg(&path)
            .status()
            .unwrap()
            .success();
        assert_eq!(is_executable(&path), expected);
    }

    #[test]
    fn nvm_fallback_uses_newest_version_containing_executable() {
        let temp = tempfile::tempdir().unwrap();
        executable(&temp.path().join("versions/node/v9.0.0/bin"));
        let expected = executable(&temp.path().join("versions/node/v25.1.0/bin"));
        std::fs::create_dir_all(temp.path().join("versions/node/v26.0.0/bin")).unwrap();
        assert_eq!(
            resolve_in("codex", OsStr::new(""), Some(temp.path())),
            Some(expected)
        );
    }

    #[test]
    fn ignores_non_executable_files_and_invalid_versions() {
        let temp = tempfile::tempdir().unwrap();
        let path = executable(&temp.path().join("versions/node/v25.1.0/bin"));
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
        executable(&temp.path().join("versions/node/not-a-version/bin"));
        assert_eq!(resolve_in("codex", OsStr::new(""), Some(temp.path())), None);
        assert_eq!(
            resolve_in("../codex", OsStr::new(""), Some(temp.path())),
            None
        );
    }

    #[test]
    fn fallback_preserves_node_and_arguments_with_quoted_paths() {
        let temp = tempfile::tempdir().unwrap();
        let fallback = executable(&temp.path().join("node's bin"));
        std::fs::write(
            &fallback,
            "#!/bin/sh\nprintf '%s\\n' \"$@\"\ncommand -v node\n",
        )
        .unwrap();
        let current = temp.path().join("current/bin");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::copy(&fallback, current.join("node")).unwrap();
        let path = std::env::join_paths([current.clone(), PathBuf::from("/usr/bin")]).unwrap();
        let command = format!(
            "{}exec codex resume 'thread id'",
            setup_for("codex", &fallback)
        );
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &command])
            .env("PATH", &path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output);
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("resume\nthread id\n{}\n", current.join("node").display())
        );
        let existing = executable(&current);
        let output = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                &format!("{}command -v codex", setup_for("codex", &fallback)),
            ])
            .env("PATH", path)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            existing.to_str().unwrap()
        );
    }
}
