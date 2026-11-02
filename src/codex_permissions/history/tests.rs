use super::*;
use std::os::unix::fs::PermissionsExt;

const THREAD: &str = "019d1af9-a899-7df1-8f7d-a244126e5ded";

struct Fixture {
    root: tempfile::TempDir,
    home: PathBuf,
    cwd: PathBuf,
    database: PathBuf,
    rollout: PathBuf,
    backups: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("codex");
        let cwd = root.path().join("project");
        fs::create_dir_all(home.join("sessions")).unwrap();
        fs::create_dir(&cwd).unwrap();
        let rollout = home.join("sessions/rollout.jsonl");
        fs::write(&rollout, b"original conversation bytes\n").unwrap();
        let database = home.join(STATE_DATABASE);
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY,
            model_provider TEXT NOT NULL, cwd TEXT NOT NULL,
            rollout_path TEXT NOT NULL, archived INTEGER NOT NULL,
            title TEXT NOT NULL)",
            )
            .unwrap();
        for id in [THREAD, "other-thread"] {
            connection
                .execute(
                    "INSERT INTO threads VALUES (?1, 'old', ?2, ?3, 0, 'original title')",
                    params![id, cwd.to_str().unwrap(), rollout.to_str().unwrap()],
                )
                .unwrap();
        }
        let backups = root.path().join("backups");
        Self {
            root,
            home,
            cwd,
            database,
            rollout,
            backups,
        }
    }

    fn history(&self) -> HistoryThread {
        HistoryThread {
            id: THREAD.into(),
            model_provider: "old".into(),
            status: HistoryStatus {
                kind: "notLoaded".into(),
            },
            cwd: self.cwd.clone(),
            path: Some(self.rollout.clone()),
        }
    }

    fn migrate(&self, thread: HistoryThread) -> Result<ProviderChange> {
        migrate(
            &self.database,
            &self.home,
            &self.backups,
            thread,
            self.cwd.to_str().unwrap(),
            "current",
        )
    }

    fn provider(&self, thread: &str) -> String {
        Connection::open(&self.database)
            .unwrap()
            .query_row(
                "SELECT model_provider FROM threads WHERE id = ?1",
                [thread],
                |row| row.get(0),
            )
            .unwrap()
    }
}

#[test]
fn migration_preserves_history_identity_and_other_threads() {
    let fixture = Fixture::new();
    let original = fs::read(&fixture.rollout).unwrap();
    let change = fixture.migrate(fixture.history()).unwrap();
    assert_eq!(fixture.provider(THREAD), "current");
    assert_eq!(fixture.provider("other-thread"), "old");
    assert_eq!(fs::read(&fixture.rollout).unwrap(), original);
    let backup: Value = serde_json::from_slice(&fs::read(&change.backup).unwrap()).unwrap();
    assert_eq!(backup["previous_provider"], "old");
    assert_eq!(backup["provider"], "current");
    assert_eq!(
        fs::metadata(&change.backup).unwrap().permissions().mode() & 0o777,
        0o600
    );
    change.rollback().unwrap();
    assert_eq!(fixture.provider(THREAD), "old");
}

#[test]
fn migration_refuses_changed_provider_wrong_directory_and_rollout() {
    let fixture = Fixture::new();
    let mut thread = fixture.history();
    thread.model_provider = "stale".into();
    assert!(fixture.migrate(thread).is_err());
    let mut thread = fixture.history();
    thread.cwd = fixture.home.clone();
    assert!(fixture.migrate(thread).is_err());
    let mut thread = fixture.history();
    thread.path = Some(fixture.database.clone());
    assert!(fixture.migrate(thread).is_err());
    assert_eq!(fixture.provider(THREAD), "old");
}

#[test]
fn migration_refuses_missing_archived_or_foreign_history() {
    let fixture = Fixture::new();
    let mut thread = fixture.history();
    thread.id = uuid::Uuid::new_v4().to_string();
    assert!(fixture.migrate(thread).is_err());
    let connection = Connection::open(&fixture.database).unwrap();
    connection
        .execute("UPDATE threads SET archived = 1 WHERE id = ?1", [THREAD])
        .unwrap();
    assert!(fixture.migrate(fixture.history()).is_err());
    let path = fixture.root.path().join("foreign.jsonl");
    fs::write(&path, "foreign").unwrap();
    let mut thread = fixture.history();
    thread.path = Some(path);
    assert!(fixture.migrate(thread).is_err());
}

#[test]
fn rollback_and_parallel_migration_do_not_overwrite_newer_changes() {
    let fixture = Fixture::new();
    let change = fixture.migrate(fixture.history()).unwrap();
    assert!(fixture.migrate(fixture.history()).is_err());
    Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE threads SET model_provider = 'newer' WHERE id = ?1",
            [THREAD],
        )
        .unwrap();
    assert!(change.rollback().is_err());
    assert_eq!(fixture.provider(THREAD), "newer");
}

#[test]
fn loaded_threads_and_non_local_endpoints_are_never_migrated() {
    let fixture = Fixture::new();
    for status in ["idle", "active", "systemError"] {
        let mut thread = fixture.history();
        thread.status.kind = status.into();
        assert!(prepare(
            History { thread },
            THREAD,
            fixture.cwd.to_str().unwrap(),
            "current",
            &serde_json::json!({}),
            true
        )
        .is_err());
    }
    assert!(prepare(
        History {
            thread: fixture.history()
        },
        THREAD,
        fixture.cwd.to_str().unwrap(),
        "current",
        &serde_json::json!({}),
        false
    )
    .is_err());
    assert_eq!(fixture.provider(THREAD), "old");
}

#[test]
fn unchanged_provider_does_not_require_a_local_database() {
    let fixture = Fixture::new();
    let mut thread = fixture.history();
    thread.status.kind = "active".into();
    thread.path = None;
    assert!(prepare(
        History { thread },
        THREAD,
        "/missing",
        "old",
        &serde_json::json!({}),
        false
    )
    .unwrap()
    .is_none());
}

#[test]
fn configured_database_directory_and_unknown_schema_fail_closed() {
    let fixture = Fixture::new();
    let config = serde_json::json!({"config": {"sqlite_home": fixture.home}});
    assert_eq!(
        database_path(&fixture.home, &config).unwrap(),
        fixture.database
    );
    assert!(database_path(
        &fixture.home,
        &serde_json::json!({"config": {"sqlite_home": "relative"}})
    )
    .is_err());
    fs::write(fixture.home.join("state_6.sqlite"), "").unwrap();
    assert!(database_path(&fixture.home, &config).is_err());
}

fn inspection(fixture: &Fixture, status: &str) -> Value {
    serde_json::json!({"thread": {"id": THREAD, "modelProvider": "current",
        "cwd": fixture.cwd, "path": fixture.rollout, "status": {"type": status}}})
}

#[test]
fn failed_resume_rolls_back_only_when_the_server_confirms_unloaded() {
    let fixture = Fixture::new();
    let change = fixture.migrate(fixture.history()).unwrap();
    let result = super::super::finish_provider_resume(
        Some(change),
        Err(anyhow::anyhow!("resume rejected")),
        Ok(inspection(&fixture, "notLoaded")),
    );
    assert!(format!("{:#}", result.unwrap_err()).contains("resume rejected"));
    assert_eq!(fixture.provider(THREAD), "old");
}

#[test]
fn unknown_or_loaded_resume_outcome_retains_provider_and_reports_backup() {
    for status in ["idle", "active"] {
        let fixture = Fixture::new();
        let change = fixture.migrate(fixture.history()).unwrap();
        let backup = change.backup.clone();
        let result = super::super::finish_provider_resume(
            Some(change),
            Err(anyhow::anyhow!("disconnected")),
            Ok(inspection(&fixture, status)),
        );
        let error = format!("{:#}", result.unwrap_err());
        assert!(error.contains("provider retained"));
        assert!(error.contains(backup.to_str().unwrap()));
        assert_eq!(fixture.provider(THREAD), "current");
    }
    let fixture = Fixture::new();
    let change = fixture.migrate(fixture.history()).unwrap();
    let result = super::super::finish_provider_resume(
        Some(change),
        Err(anyhow::anyhow!("timed out")),
        Err(anyhow::anyhow!("server unavailable")),
    );
    assert!(format!("{:#}", result.unwrap_err()).contains("server unavailable"));
    assert_eq!(fixture.provider(THREAD), "current");
}

#[test]
fn failed_backup_never_changes_metadata_and_cancellation_never_blindly_rolls_back() {
    let fixture = Fixture::new();
    fs::write(&fixture.backups, "not a directory").unwrap();
    assert!(fixture.migrate(fixture.history()).is_err());
    assert_eq!(fixture.provider(THREAD), "old");
    fs::remove_file(&fixture.backups).unwrap();
    drop(fixture.migrate(fixture.history()).unwrap());
    assert_eq!(fixture.provider(THREAD), "current");
}
