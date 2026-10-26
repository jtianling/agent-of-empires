use super::{Connection, Store, MAX_SLOT};
use anyhow::{ensure, Context, Result};
use std::path::{Path, PathBuf};

impl Store {
    pub(crate) fn open_read_only(profile: &str) -> Result<Option<Self>> {
        let path = crate::session::get_app_dir_path()?
            .join("profiles")
            .join(profile)
            .join("aoe.db");
        if stamp(&path)?.is_none() {
            return Ok(None);
        }
        let temporary = tempfile::tempdir().context("Creating private inventory snapshot")?;
        let copied = temporary.path().join("aoe.db");
        snapshot(&path, &copied)?;
        let conn = Connection::open_with_flags(
            &copied,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Ok(Some(Self {
            conn,
            _inventory_snapshot: Some(temporary),
        }))
    }

    pub(crate) fn inventory_slots(
        &self,
        instance_id: &str,
    ) -> Result<Vec<(i64, String, String, String)>> {
        let mut statement = self.conn.prepare(
            "SELECT slot, agent, cwd, tmux_pane FROM agent_slot \
             WHERE instance_id = ?1 ORDER BY slot",
        )?;
        let rows = statement.query_map([instance_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        let result: Vec<(i64, String, String, String)> =
            rows.collect::<rusqlite::Result<Vec<_>>>()?;
        for (slot, _, _, pane) in &result {
            anyhow::ensure!((0..=MAX_SLOT).contains(slot), "Invalid stored pane slot");
            anyhow::ensure!(
                pane.is_empty()
                    || pane.strip_prefix('%').is_some_and(
                        |id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())
                    ),
                "Invalid stored tmux pane ID"
            );
        }
        Ok(result)
    }
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn snapshot(source: &Path, destination: &Path) -> Result<()> {
    let wal = sidecar(source, "-wal");
    let before = (stamp(source)?, stamp(&wal)?);
    std::fs::copy(source, destination).context("Copying inventory database snapshot")?;
    if before.1.is_some() {
        std::fs::copy(&wal, sidecar(destination, "-wal"))
            .context("Copying inventory WAL snapshot")?;
    }
    ensure!(
        before == (stamp(source)?, stamp(&wal)?),
        "Session database changed during inventory snapshot; retry query"
    );
    Ok(())
}

fn stamp(path: &Path) -> Result<Option<(u64, std::time::SystemTime)>> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(Some((metadata.len(), metadata.modified()?))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("Inspecting database snapshot source"),
    }
}
