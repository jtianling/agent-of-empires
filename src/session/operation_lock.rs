use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use anyhow::{Context, Result};
use fs2::FileExt;

thread_local! {
    static LOCKS: RefCell<HashMap<PathBuf, Weak<File>>> = RefCell::new(HashMap::new());
}

pub struct OperationGuard {
    _file: Rc<File>,
}

pub(super) fn acquire(path: &Path) -> Result<OperationGuard> {
    acquire_inner(path, true)?.context("Managed operation lock is unavailable")
}

pub(super) fn try_acquire(path: &Path) -> Result<Option<OperationGuard>> {
    acquire_inner(path, false)
}

fn acquire_inner(path: &Path, blocking: bool) -> Result<Option<OperationGuard>> {
    let parent = path.parent().context("Missing operation lock directory")?;
    fs::create_dir_all(parent)?;
    let path = parent
        .canonicalize()?
        .join(path.file_name().context("Missing lock name")?);
    if let Some(file) = LOCKS.with(|locks| locks.borrow().get(&path).and_then(Weak::upgrade)) {
        return Ok(Some(OperationGuard { _file: file }));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    if blocking {
        file.lock_exclusive().context("Locking managed operation")?;
    } else if let Err(error) = file.try_lock_exclusive() {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            return Ok(None);
        }
        return Err(error).context("Trying managed operation lock");
    }
    let file = Rc::new(file);
    LOCKS.with(|locks| {
        locks.borrow_mut().insert(path, Rc::downgrade(&file));
    });
    Ok(Some(OperationGuard { _file: file }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available_on_another_thread(path: &Path) -> bool {
        let path = path.to_path_buf();
        std::thread::spawn(move || try_acquire(&path).unwrap().is_some())
            .join()
            .unwrap()
    }

    #[test]
    fn nonblocking_lock_is_busy_until_all_reentrant_guards_drop() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("operation.lock");
        let outer = acquire(&path).unwrap();
        let inner = try_acquire(&path).unwrap().unwrap();
        assert!(!available_on_another_thread(&path));
        drop(outer);
        assert!(!available_on_another_thread(&path));
        drop(inner);
        assert!(available_on_another_thread(&path));
    }
}
