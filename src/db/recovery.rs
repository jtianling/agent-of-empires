use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};

use super::Store;

impl Store {
    pub(crate) fn instance_ids_with_slots(profile: &str) -> Result<HashSet<String>> {
        let path = crate::session::get_app_dir_path()?
            .join("profiles")
            .join(profile)
            .join("aoe.db");
        read_instance_ids(&path)
    }
}

fn read_instance_ids(path: &Path) -> Result<HashSet<String>> {
    match std::fs::metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(error) => return Err(error).context("Inspecting recovery database"),
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("Opening recovery database read-only")?;
    conn.busy_timeout(std::time::Duration::ZERO)?;
    let mut statement = conn.prepare("SELECT DISTINCT instance_id FROM agent_slot")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect::<rusqlite::Result<HashSet<_>>>()
        .context("Reading recoverable session identities")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_committed_slots_while_a_writer_is_active() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("aoe.db");
        let (store, _) = Store::open_with_schema_at(&path).unwrap();
        store
            .conn
            .execute(
                "INSERT INTO agent_slot (instance_id, slot, agent, native_session_id, cwd, \
             last_seen_at) VALUES ('committed', 0, 'shell', '', '/tmp', 1)",
                [],
            )
            .unwrap();
        store
            .conn
            .execute_batch(
                "BEGIN IMMEDIATE; INSERT INTO agent_slot \
             (instance_id, slot, agent, native_session_id, cwd, last_seen_at) \
             VALUES ('uncommitted', 0, 'shell', '', '/tmp', 1);",
            )
            .unwrap();
        assert_eq!(
            read_instance_ids(&path).unwrap(),
            HashSet::from(["committed".into()])
        );
        store.conn.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn missing_or_invalid_database_is_not_created_or_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("aoe.db");
        assert!(read_instance_ids(&path).unwrap().is_empty());
        assert!(!path.exists());
        std::fs::write(&path, b"invalid database").unwrap();
        assert!(read_instance_ids(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid database");
    }
}
