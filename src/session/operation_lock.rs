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
    let parent = path.parent().context("Missing operation lock directory")?;
    fs::create_dir_all(parent)?;
    let path = parent
        .canonicalize()?
        .join(path.file_name().context("Missing lock name")?);
    if let Some(file) = LOCKS.with(|locks| locks.borrow().get(&path).and_then(Weak::upgrade)) {
        return Ok(OperationGuard { _file: file });
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    file.lock_exclusive().context("Locking managed operation")?;
    let file = Rc::new(file);
    LOCKS.with(|locks| {
        locks.borrow_mut().insert(path, Rc::downgrade(&file));
    });
    Ok(OperationGuard { _file: file })
}
