use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const STATE_DATABASE: &str = "state_5.sqlite";

#[cfg(test)]
mod tests;

#[derive(Deserialize)]
pub(super) struct History {
    pub thread: HistoryThread,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HistoryThread {
    pub id: String,
    pub model_provider: String,
    pub status: HistoryStatus,
    pub cwd: PathBuf,
    pub path: Option<PathBuf>,
}

#[derive(Deserialize)]
pub(super) struct HistoryStatus {
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Serialize)]
struct Backup<'a> {
    thread_id: &'a str,
    database: &'a Path,
    previous_provider: &'a str,
    provider: &'a str,
}

pub(super) struct ProviderChange {
    _lock: File,
    database: PathBuf,
    thread_id: String,
    previous: String,
    provider: String,
    backup: PathBuf,
    finished: bool,
}

impl ProviderChange {
    pub fn commit(mut self) {
        self.finished = true;
    }

    pub fn rollback(mut self) -> Result<()> {
        self.finished = true;
        let result = self.restore_record().with_context(|| {
            format!(
                "Restoring Codex history provider; backup: {}",
                self.backup.display()
            )
        });
        if let Err(error) = &result {
            tracing::error!("{error:#}");
        }
        result
    }

    fn restore_record(&self) -> Result<()> {
        let connection = open_database(&self.database)?;
        let changed = connection.execute(
            "UPDATE threads SET model_provider = ?1 WHERE id = ?2 AND model_provider = ?3",
            params![self.previous, self.thread_id, self.provider],
        )?;
        ensure!(changed == 1, "Codex provider changed before rollback");
        tracing::warn!(thread_id = self.thread_id, backup = %self.backup.display(),
            "Rolled back Codex history provider after failed resume");
        Ok(())
    }

    pub fn is_unloaded(&self, response: Value) -> Result<bool> {
        let history: History = serde_json::from_value(response)
            .context("Invalid Codex thread response after failed resume")?;
        ensure!(
            history.thread.id == self.thread_id,
            "Codex returned another rollback thread"
        );
        Ok(history.thread.status.kind == "notLoaded")
    }

    pub fn retain(mut self, error: anyhow::Error) -> anyhow::Error {
        self.finished = true;
        let error = error.context(format!(
            "Codex resume outcome is unconfirmed; history provider retained; backup: {}",
            self.backup.display()
        ));
        tracing::error!("{error:#}");
        error
    }
}

impl Drop for ProviderChange {
    fn drop(&mut self) {
        if !self.finished {
            tracing::error!(thread_id = self.thread_id, backup = %self.backup.display(),
                "Codex resume canceled after provider migration; outcome unknown, history provider retained");
        }
    }
}

pub(super) fn prepare(
    history: History,
    thread: &str,
    cwd: &str,
    provider: &str,
    configuration: &Value,
    local: bool,
) -> Result<Option<ProviderChange>> {
    ensure!(
        history.thread.id == thread,
        "Codex returned another history thread"
    );
    if history.thread.model_provider == provider {
        return Ok(None);
    }
    ensure!(
        local,
        "Cannot migrate Codex history on a non-local app-server"
    );
    ensure!(
        history.thread.status.kind == "notLoaded",
        "Codex history provider differs but the thread is still loaded; close its other clients before resuming"
    );
    let home = codex_home()?;
    let database = database_path(&home, configuration)?;
    let backups = crate::session::get_app_dir_path()?.join("codex-provider-backups");
    migrate(&database, &home, &backups, history.thread, cwd, provider).map(Some)
}

fn codex_home() -> Result<PathBuf> {
    let home = match std::env::var_os("CODEX_HOME") {
        Some(home) => {
            ensure!(!home.is_empty(), "CODEX_HOME must not be empty");
            PathBuf::from(home)
        }
        None => dirs::home_dir()
            .context("Missing home directory")?
            .join(".codex"),
    };
    home.canonicalize().context("Resolving Codex history home")
}

fn database_path(home: &Path, configuration: &Value) -> Result<PathBuf> {
    let directory = match configuration.pointer("/config/sqlite_home") {
        None | Some(Value::Null) => home.to_path_buf(),
        Some(Value::String(path)) => {
            let path = PathBuf::from(path);
            ensure!(path.is_absolute(), "Codex sqlite_home must be absolute");
            path
        }
        Some(_) => anyhow::bail!("Invalid Codex sqlite_home configuration"),
    };
    for entry in fs::read_dir(&directory).context("Reading Codex database directory")? {
        let name = entry?.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("state_") && name.ends_with(".sqlite") && name != STATE_DATABASE {
            anyhow::bail!("Unsupported Codex state database {name}; history was not changed");
        }
    }
    Ok(directory.join(STATE_DATABASE))
}

fn open_database(path: &Path) -> Result<Connection> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .with_context(|| format!("Opening Codex history database {}", path.display()))?;
    connection.busy_timeout(std::time::Duration::from_secs(2))?;
    Ok(connection)
}

fn migrate(
    database: &Path,
    home: &Path,
    backups: &Path,
    thread: HistoryThread,
    cwd: &str,
    provider: &str,
) -> Result<ProviderChange> {
    uuid::Uuid::try_parse(&thread.id).context("Invalid Codex history UUID")?;
    let cwd = Path::new(cwd)
        .canonicalize()
        .context("Resolving resume directory")?;
    ensure!(
        thread.cwd.canonicalize()? == cwd,
        "Codex history belongs to another directory"
    );
    let rollout = thread
        .path
        .as_ref()
        .context("Codex history has no local rollout path")?
        .canonicalize()
        .context("Resolving Codex rollout")?;
    let home = home
        .canonicalize()
        .context("Resolving Codex history home")?;
    ensure!(
        rollout.starts_with(home.join("sessions")),
        "Codex history is outside the local sessions directory"
    );
    let mut connection = open_database(database)?;
    let lock = lock_thread(backups, &thread.id)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    validate_record(&transaction, &thread, &cwd, &rollout)?;
    let backup = write_backup(backups, database, &thread, provider)?;
    let changed = transaction.execute(
        "UPDATE threads SET model_provider = ?1 WHERE id = ?2 AND model_provider = ?3",
        params![provider, thread.id, thread.model_provider],
    )?;
    ensure!(
        changed == 1,
        "Codex history provider changed before migration"
    );
    transaction
        .commit()
        .context("Committing Codex history provider migration")?;
    tracing::info!(thread_id = thread.id, previous_provider = thread.model_provider, provider,
        backup = %backup.display(), "Aligned Codex history with the current provider");
    Ok(ProviderChange {
        _lock: lock,
        database: database.to_path_buf(),
        thread_id: thread.id,
        previous: thread.model_provider,
        provider: provider.to_string(),
        backup,
        finished: false,
    })
}

fn lock_thread(directory: &Path, thread: &str) -> Result<File> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    let path = directory.join(format!("{thread}.lock"));
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .context("Another AoE operation is changing this Codex provider")?;
    Ok(lock)
}

fn validate_record(
    connection: &Connection,
    thread: &HistoryThread,
    cwd: &Path,
    rollout: &Path,
) -> Result<()> {
    let (provider, stored_cwd, path, archived): (String, String, String, bool) = connection
        .query_row(
            "SELECT model_provider, cwd, rollout_path, archived FROM threads WHERE id = ?1",
            [&thread.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .context("Reading the exact Codex history record")?;
    ensure!(!archived, "Cannot migrate archived Codex history");
    ensure!(
        provider == thread.model_provider,
        "Codex history provider changed during inspection"
    );
    ensure!(
        Path::new(&stored_cwd).canonicalize()? == cwd,
        "Codex database directory mismatch"
    );
    ensure!(
        Path::new(&path).canonicalize()? == rollout,
        "Codex database rollout mismatch"
    );
    Ok(())
}

fn write_backup(
    directory: &Path,
    database: &Path,
    thread: &HistoryThread,
    provider: &str,
) -> Result<PathBuf> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    let path = directory.join(format!("{}-{}.json", thread.id, uuid::Uuid::new_v4()));
    let backup = Backup {
        thread_id: &thread.id,
        database,
        previous_provider: &thread.model_provider,
        provider,
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(&serde_json::to_vec(&backup)?)?;
    file.sync_all()
        .context("Saving Codex provider backup before migration")?;
    Ok(path)
}
