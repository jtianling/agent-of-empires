//! Session storage with optimistic concurrency and atomic replacement.

use anyhow::{Context, Result};
use fs2::FileExt;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::fs::{self, File, OpenOptions};
use std::path::PathBuf;
use tracing::warn;

use super::storage_merge::{atomic_write, merge};
use super::{get_profile_dir, Group, GroupTree, Instance, DEFAULT_PROFILE};

pub struct Storage {
    profile: String,
    sessions_path: PathBuf,
    baseline: RefCell<Vec<Value>>,
    group_baseline: RefCell<Vec<Value>>,
    locked: Cell<bool>,
}

pub struct StorageGuard<'a> {
    storage: &'a Storage,
    _file: File,
}

impl Drop for StorageGuard<'_> {
    fn drop(&mut self) {
        self.storage.locked.set(false);
    }
}

impl Storage {
    pub fn new(profile: &str) -> Result<Self> {
        let profile = if profile.is_empty() {
            DEFAULT_PROFILE
        } else {
            profile
        };
        Ok(Self {
            profile: profile.to_string(),
            sessions_path: get_profile_dir(profile)?.join("sessions.json"),
            baseline: RefCell::new(Vec::new()),
            group_baseline: RefCell::new(Vec::new()),
            locked: Cell::new(false),
        })
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }

    pub fn lock(&self) -> Result<StorageGuard<'_>> {
        anyhow::ensure!(!self.locked.get(), "Storage is already locked");
        let parent = self
            .sessions_path
            .parent()
            .context("Missing storage directory")?;
        fs::create_dir_all(parent)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(parent.join("sessions.lock"))?;
        file.lock_exclusive().context("Locking session storage")?;
        self.locked.set(true);
        Ok(StorageGuard {
            storage: self,
            _file: file,
        })
    }

    pub fn operation_lock(&self) -> Result<super::operation_lock::OperationGuard> {
        super::operation_lock::acquire(&self.sessions_path.with_file_name("managed-operation.lock"))
    }

    fn read_instances(&self) -> Result<Vec<Instance>> {
        let mut instances: Vec<Instance> = self.read_json(&self.sessions_path)?;
        for instance in &mut instances {
            instance.hydrate_legacy_primary_pane();
        }
        Ok(instances)
    }

    fn read_json<T: serde::de::DeserializeOwned>(&self, path: &std::path::Path) -> Result<Vec<T>> {
        let content = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        if content.trim().is_empty() {
            return Ok(Vec::new());
        }
        serde_json::from_str(&content).with_context(|| format!("Reading {}", path.display()))
    }

    pub fn load(&self) -> Result<Vec<Instance>> {
        let _guard = if self.locked.get() {
            None
        } else {
            Some(self.lock()?)
        };
        let instances = self.read_instances()?;
        *self.baseline.borrow_mut() = values(&instances)?;
        Ok(instances)
    }

    pub fn load_with_groups(&self) -> Result<(Vec<Instance>, Vec<Group>)> {
        let _guard = if self.locked.get() {
            None
        } else {
            Some(self.lock()?)
        };
        let instances = self.load()?;
        let groups = self.read_json(&self.sessions_path.with_file_name("groups.json"))?;
        *self.group_baseline.borrow_mut() = values(&groups)?;
        Ok((instances, groups))
    }

    pub fn save(&self, instances: &[Instance]) -> Result<()> {
        self.save_inner(instances, None)
    }

    pub fn save_with_groups(&self, instances: &[Instance], tree: &GroupTree) -> Result<()> {
        self.save_inner(instances, Some(tree.get_all_groups()))
    }

    fn save_inner(&self, instances: &[Instance], groups: Option<Vec<Group>>) -> Result<()> {
        let _guard = if self.locked.get() {
            None
        } else {
            Some(self.lock()?)
        };
        let next = values(instances)?;
        let current = values(&self.read_instances()?)?;
        let merged = merge(&self.baseline.borrow(), &current, &next, "id")?;
        let group_path = self.sessions_path.with_file_name("groups.json");
        let group_update = groups
            .map(|groups| -> Result<_> {
                let next = values(&groups)?;
                let current: Vec<Value> = self.read_json(&group_path)?;
                let merged = merge(&self.group_baseline.borrow(), &current, &next, "path")?;
                Ok((next, merged))
            })
            .transpose()?;
        self.backup()?;
        atomic_write(&self.sessions_path, &serde_json::to_vec_pretty(&merged)?)?;
        if let Some((next_groups, merged_groups)) = group_update {
            let write = atomic_write(&group_path, &serde_json::to_vec_pretty(&merged_groups)?);
            if let Err(error) = write {
                atomic_write(&self.sessions_path, &serde_json::to_vec_pretty(&current)?)
                    .with_context(|| {
                        format!("Group save failed: {error:#}; session rollback failed")
                    })?;
                return Err(error.context("Group save failed; session changes rolled back"));
            }
            *self.group_baseline.borrow_mut() = next_groups;
        }
        *self.baseline.borrow_mut() = next;
        Ok(())
    }

    fn backup(&self) -> Result<()> {
        if self.sessions_path.exists() {
            if let Err(error) = fs::copy(
                &self.sessions_path,
                self.sessions_path.with_extension("json.bak"),
            ) {
                warn!("Failed to create backup: {error}");
            }
        }
        Ok(())
    }
}

fn values<T: serde::Serialize>(items: &[T]) -> Result<Vec<Value>> {
    items
        .iter()
        .map(|item| serde_json::to_value(item).map_err(Into::into))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::tempdir;

    fn setup_test_home(temp: &std::path::Path) -> crate::session::TestHomeGuard {
        crate::session::scoped_test_home(temp)
    }

    #[test]
    #[serial]
    fn concurrent_writers_preserve_creation_and_deletion() -> Result<()> {
        let temp = tempdir()?;
        let _home = crate::session::scoped_test_home(temp.path());
        let first = Storage::new("concurrent")?;
        let second = Storage::new("concurrent")?;
        let original = Instance::new("original", "/tmp");
        first.save(std::slice::from_ref(&original))?;
        let mut local = first.load()?;
        let mut remote = second.load()?;
        let added = Instance::new("added", "/tmp");
        remote.push(added.clone());
        second.save(&remote)?;
        local[0].title = "changed".into();
        first.save(&local)?;
        let reader = Storage::new("concurrent")?;
        let merged = reader.load()?;
        assert_eq!(merged.len(), 2);
        assert!(merged.iter().any(|instance| instance.id == added.id));
        second.load()?;
        second.save(&[])?;
        first.save(&local)?;
        assert!(reader.load()?.is_empty());
        Ok(())
    }

    #[test]
    #[serial]
    fn test_storage_roundtrip() -> Result<()> {
        let temp = tempdir()?;
        let _home = setup_test_home(temp.path());

        let storage = Storage::new("test-profile")?;

        let instances = vec![
            Instance::new("test1", "/tmp/test1"),
            Instance::new("test2", "/tmp/test2"),
        ];

        storage.save(&instances)?;
        let loaded = storage.load()?;

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].title, "test1");
        assert_eq!(loaded[1].title, "test2");

        Ok(())
    }

    #[test]
    #[serial]
    fn test_storage_new_with_empty_profile() -> Result<()> {
        let temp = tempdir()?;
        let _home = setup_test_home(temp.path());

        let storage = Storage::new("")?;
        assert_eq!(storage.profile(), "default");
        Ok(())
    }

    #[test]
    #[serial]
    fn test_storage_new_with_custom_profile() -> Result<()> {
        let temp = tempdir()?;
        let _home = setup_test_home(temp.path());

        let storage = Storage::new("custom-profile")?;
        assert_eq!(storage.profile(), "custom-profile");
        Ok(())
    }

    #[test]
    #[serial]
    fn test_storage_load_nonexistent_file() -> Result<()> {
        let temp = tempdir()?;
        let _home = setup_test_home(temp.path());

        let storage = Storage::new("test-empty")?;
        let loaded = storage.load()?;

        assert!(loaded.is_empty());
        Ok(())
    }

    #[test]
    #[serial]
    fn test_storage_load_empty_file() -> Result<()> {
        let temp = tempdir()?;
        let _home = setup_test_home(temp.path());

        let storage = Storage::new("test-empty-file")?;

        // Create empty file
        fs::create_dir_all(storage.sessions_path.parent().unwrap())?;
        fs::write(&storage.sessions_path, "")?;

        let loaded = storage.load()?;
        assert!(loaded.is_empty());
        Ok(())
    }

    #[test]
    #[serial]
    fn test_storage_load_whitespace_only_file() -> Result<()> {
        let temp = tempdir()?;
        let _home = setup_test_home(temp.path());

        let storage = Storage::new("test-whitespace")?;

        fs::create_dir_all(storage.sessions_path.parent().unwrap())?;
        fs::write(&storage.sessions_path, "   \n  \t  ")?;

        let loaded = storage.load()?;
        assert!(loaded.is_empty());
        Ok(())
    }

    #[test]
    #[serial]
    fn test_storage_save_creates_backup() -> Result<()> {
        let temp = tempdir()?;
        let _home = setup_test_home(temp.path());

        let storage = Storage::new("test-backup")?;

        // First save
        let instances = vec![Instance::new("test1", "/tmp/test1")];
        storage.save(&instances)?;

        // Second save (should create backup)
        let instances2 = vec![Instance::new("test2", "/tmp/test2")];
        storage.save(&instances2)?;

        // Check backup exists
        let backup_path = storage.sessions_path.with_extension("json.bak");
        assert!(backup_path.exists());

        // Backup should contain first save content
        let backup_content = fs::read_to_string(&backup_path)?;
        assert!(backup_content.contains("test1"));
        Ok(())
    }

    #[test]
    #[serial]
    fn test_storage_save_empty_array() -> Result<()> {
        let temp = tempdir()?;
        let _home = setup_test_home(temp.path());

        let storage = Storage::new("test-empty-save")?;
        storage.save(&[])?;

        let content = fs::read_to_string(&storage.sessions_path)?;
        assert_eq!(content.trim(), "[]");
        Ok(())
    }

    #[test]
    #[serial]
    fn test_storage_load_with_groups_no_groups_file() -> Result<()> {
        let temp = tempdir()?;
        let _home = setup_test_home(temp.path());

        let storage = Storage::new("test-no-groups")?;

        let instances = vec![Instance::new("test", "/tmp/test")];
        storage.save(&instances)?;

        let (loaded_instances, loaded_groups) = storage.load_with_groups()?;
        assert_eq!(loaded_instances.len(), 1);
        assert!(loaded_groups.is_empty());
        Ok(())
    }

    #[test]
    #[serial]
    fn test_storage_save_and_load_with_groups() -> Result<()> {
        let temp = tempdir()?;
        let _home = setup_test_home(temp.path());

        let storage = Storage::new("test-with-groups")?;

        let mut instances = vec![Instance::new("test", "/tmp/test")];
        instances[0].group_path = "work/projects".to_string();

        let groups = vec![Group::new("projects", "work/projects")];
        let group_tree = GroupTree::new_with_groups(&instances, &groups);

        storage.save_with_groups(&instances, &group_tree)?;

        let (loaded_instances, loaded_groups) = storage.load_with_groups()?;
        assert_eq!(loaded_instances.len(), 1);
        assert_eq!(loaded_instances[0].group_path, "work/projects");
        assert!(!loaded_groups.is_empty());
        Ok(())
    }

    #[test]
    #[serial]
    fn test_storage_load_invalid_json() -> Result<()> {
        let temp = tempdir()?;
        let _home = setup_test_home(temp.path());

        let storage = Storage::new("test-invalid")?;

        fs::create_dir_all(storage.sessions_path.parent().unwrap())?;
        fs::write(&storage.sessions_path, "{ invalid json }")?;

        let result = storage.load();
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    #[serial]
    fn test_storage_preserves_instance_fields() -> Result<()> {
        let temp = tempdir()?;
        let _home = setup_test_home(temp.path());

        let storage = Storage::new("test-fields")?;

        let mut instance = Instance::new("Test Project", "/home/user/project");
        instance.tool = "opencode".to_string();
        instance.command = "opencode --config test".to_string();
        instance.group_path = "work/clients".to_string();
        // The pane owns the tool and the instance field mirrors it, so a record
        // built from the legacy fields has to be synced the way every producer
        // does (see `cli::add`) -- otherwise the load-time hydration derives the
        // tool back from an untouched default pane.
        instance.sync_primary_pane_from_legacy();

        storage.save(&[instance.clone()])?;
        let loaded = storage.load()?;

        assert_eq!(loaded.len(), 1);
        let loaded_instance = &loaded[0];
        assert_eq!(loaded_instance.title, "Test Project");
        assert_eq!(loaded_instance.project_path, "/home/user/project");
        assert_eq!(loaded_instance.tool, "opencode");
        assert_eq!(loaded_instance.command, "opencode --config test");
        assert_eq!(loaded_instance.group_path, "work/clients");
        Ok(())
    }

    #[test]
    #[serial]
    fn test_storage_profile_accessor() -> Result<()> {
        let temp = tempdir()?;
        let _home = setup_test_home(temp.path());

        // Verify profiles are correctly named
        let storage1 = Storage::new("profile-alpha")?;
        let storage2 = Storage::new("profile-beta")?;

        assert_eq!(storage1.profile(), "profile-alpha");
        assert_eq!(storage2.profile(), "profile-beta");

        // Verify they use different paths (implying isolation)
        assert_ne!(storage1.sessions_path, storage2.sessions_path);
        Ok(())
    }

    #[test]
    #[serial]
    fn test_storage_groups_file_empty() -> Result<()> {
        let temp = tempdir()?;
        let _home = setup_test_home(temp.path());

        let storage = Storage::new("test-empty-groups")?;

        // Save sessions
        storage.save(&[Instance::new("test", "/tmp/test")])?;

        // Create empty groups file
        let groups_path = storage.sessions_path.with_file_name("groups.json");
        fs::write(&groups_path, "   ")?;

        let (instances, groups) = storage.load_with_groups()?;
        assert_eq!(instances.len(), 1);
        assert!(groups.is_empty());
        Ok(())
    }
}
