use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::Value;

pub(super) fn merge(
    baseline: &[Value],
    current: &[Value],
    requested: &[Value],
    key: &str,
) -> Result<Vec<Value>> {
    let old = indexed(baseline, key)?;
    let disk = indexed(current, key)?;
    let next = indexed(requested, key)?;
    let mut merged = current.to_vec();
    for (id, before) in &old {
        let after = next.get(id).copied();
        if after == Some(*before) {
            continue;
        }
        let actual = disk.get(id).copied();
        if actual != Some(*before) && actual != after {
            bail!("Concurrent modification of {key} '{id}'; reload and retry");
        }
        match after {
            Some(after) => {
                if let Some(entry) = merged
                    .iter_mut()
                    .find(|entry| entry[key].as_str() == Some(id))
                {
                    *entry = after.clone();
                }
            }
            None => merged.retain(|entry| entry[key].as_str() != Some(id)),
        }
    }
    for after in requested {
        let id = after[key].as_str().context("Missing record identity")?;
        if old.contains_key(id) {
            continue;
        }
        match disk.get(id) {
            Some(actual) if *actual != after => {
                bail!("Concurrent creation of {key} '{id}'; reload and retry");
            }
            Some(_) => {}
            None => merged.push(after.clone()),
        }
    }
    let previous_order: Vec<_> = baseline
        .iter()
        .filter_map(|entry| entry[key].as_str())
        .collect();
    let requested_order: Vec<_> = requested
        .iter()
        .filter_map(|entry| entry[key].as_str())
        .collect();
    if previous_order != requested_order {
        let order: HashMap<_, _> = requested
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry[key].as_str().map(|id| (id.to_owned(), index)))
            .collect();
        merged.sort_by_key(|entry| {
            order
                .get(entry[key].as_str().unwrap_or_default())
                .copied()
                .unwrap_or(usize::MAX)
        });
    }
    Ok(merged)
}

fn indexed<'a>(entries: &'a [Value], key: &str) -> Result<HashMap<&'a str, &'a Value>> {
    let mut result = HashMap::new();
    for entry in entries {
        let id = entry[key].as_str().context("Missing record identity")?;
        if result.insert(id, entry).is_some() {
            bail!("Duplicate {key}: {id}");
        }
    }
    Ok(result)
}

pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        if let Err(error) = fs::remove_file(&temporary) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("Failed to clean temporary storage file: {error}");
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(id: &str, title: &str) -> Value {
        json!({"id": id, "title": title})
    }

    #[test]
    fn preserves_initial_and_requested_order() {
        let original = vec![record("b", "B"), record("a", "A")];
        assert_eq!(merge(&[], &[], &original, "id").unwrap(), original);
        let reversed = vec![original[1].clone(), original[0].clone()];
        assert_eq!(
            merge(&original, &original, &reversed, "id").unwrap(),
            reversed
        );
    }

    #[test]
    fn merges_independent_edits_and_keeps_remote_creation() {
        let original = vec![record("a", "A"), record("b", "B")];
        let current = vec![record("a", "A"), record("b", "remote"), record("c", "C")];
        let next = vec![record("a", "local"), record("b", "B")];
        assert_eq!(
            merge(&original, &current, &next, "id").unwrap(),
            vec![
                record("a", "local"),
                record("b", "remote"),
                record("c", "C")
            ]
        );
    }

    #[test]
    fn unchanged_deleted_record_does_not_resurrect() {
        let original = vec![record("a", "A")];
        assert!(merge(&original, &[], &original, "id").unwrap().is_empty());
        assert!(merge(&original, &[], &[record("a", "changed")], "id").is_err());
    }

    #[test]
    fn conflicting_owner_change_is_rejected() {
        let original = vec![json!({"id": "a", "owner": "first"})];
        let current = vec![json!({"id": "a", "owner": null})];
        let next = vec![json!({"id": "a", "owner": "first", "title": "new"})];
        assert!(merge(&original, &current, &next, "id").is_err());
    }

    #[test]
    fn unchanged_local_snapshot_preserves_remote_reordering() {
        let original = vec![record("a", "A"), record("b", "B")];
        let current = vec![original[1].clone(), original[0].clone()];
        assert_eq!(
            merge(&original, &current, &original, "id").unwrap(),
            current
        );
    }
}
