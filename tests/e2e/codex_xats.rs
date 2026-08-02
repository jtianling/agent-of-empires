use serial_test::serial;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};
use tempfile::TempDir;

use crate::harness::require_tmux;

const AGENT_ID: &str = "019d1af9-a899-7df1-8f7d-a244126e5ded";

struct CodexXatsHarness {
    home: TempDir,
    bin_dir: TempDir,
    tmux_dir: TempDir,
    socket_path: PathBuf,
    outer_session: String,
    binary: PathBuf,
    project: PathBuf,
    codex_log: PathBuf,
    xats_log: PathBuf,
    prereg_failure: PathBuf,
    app_server_failure: PathBuf,
}

impl CodexXatsHarness {
    fn new(test_name: &str) -> Self {
        let home = TempDir::new().expect("create test home");
        let bin_dir = TempDir::new().expect("create test bin");
        let tmux_dir = TempDir::new().expect("create tmux dir");
        let project = home.path().join("project with quote ' and space");
        std::fs::create_dir_all(&project).expect("create project");

        let socket_parent = tmux_dir.path().join(format!("tmux-{}", current_uid()));
        std::fs::create_dir_all(&socket_parent).expect("create socket parent");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&socket_parent, std::fs::Permissions::from_mode(0o700))
                .expect("set socket permissions");
        }

        let harness = Self {
            socket_path: socket_parent.join("default"),
            outer_session: format!("aoe_e2e_{test_name}_{}", std::process::id()),
            binary: PathBuf::from(env!("CARGO_BIN_EXE_aoe")),
            codex_log: home.path().join("codex-args.log"),
            xats_log: home.path().join("xats-args.log"),
            prereg_failure: home.path().join("fail-prereg"),
            app_server_failure: home.path().join("fail-app-server"),
            home,
            bin_dir,
            tmux_dir,
            project,
        };
        harness.write_config();
        harness.write_shims();
        harness
    }

    fn config_dir(&self) -> PathBuf {
        if cfg!(target_os = "linux") {
            self.home.path().join(".config/agent-of-empires")
        } else {
            self.home.path().join(".agent-of-empires")
        }
    }

    fn write_config(&self) {
        let config_dir = self.config_dir();
        std::fs::create_dir_all(config_dir.join("profiles/default")).expect("create profile dir");
        let config = format!(
            r#"[updates]
check_enabled = false

[app_state]
has_seen_welcome = true
last_seen_version = "{}"

[session]
default_tool = "codex"
cross_agent_team_default = true
"#,
            env!("CARGO_PKG_VERSION")
        );
        std::fs::write(config_dir.join("config.toml"), config).expect("write config");
    }

    fn write_shims(&self) {
        write_executable(
            &self.bin_dir.path().join("aoe-test-shell"),
            "#!/bin/sh\nif [ \"$1\" = \"-lc\" ]; then shift; exec /bin/sh -c \"$1\"; fi\nexec /bin/sh \"$@\"\n",
        );
        write_executable(
            &self.bin_dir.path().join("uuidgen"),
            &format!("#!/bin/sh\nprintf '%s\\n' '{AGENT_ID}'\n"),
        );
        write_executable(
            &self.bin_dir.path().join("nc"),
            "#!/bin/sh\n[ -f \"$APP_SERVER_FAILURE\" ] && exit 1\nexit 0\n",
        );
        write_executable(
            &self.bin_dir.path().join("npx"),
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$XATS_ARGS_LOG\"\n[ -f \"$PREREG_FAILURE\" ] && { printf '%s\\n' 'controlled preregistration failure' >&2; exit 17; }\nexit 0\n",
        );
        write_executable(
            &self.bin_dir.path().join("codex"),
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$CODEX_ARGS_LOG\"\nexec sleep 30\n",
        );
    }

    fn apply_env(&self, command: &mut Command) {
        let system_path = std::env::var("PATH").unwrap_or_default();
        command
            .env("HOME", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join(".config"))
            .env(
                "PATH",
                format!("{}:{system_path}", self.bin_dir.path().display()),
            )
            .env("SHELL", self.bin_dir.path().join("aoe-test-shell"))
            .env("AGENT_OF_EMPIRES_PROFILE", "default")
            .env("TMUX_TMPDIR", self.tmux_dir.path())
            .env("CODEX_ARGS_LOG", &self.codex_log)
            .env("XATS_ARGS_LOG", &self.xats_log)
            .env("PREREG_FAILURE", &self.prereg_failure)
            .env("APP_SERVER_FAILURE", &self.app_server_failure)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE");
    }

    fn tmux(&self, args: &[&str]) -> Output {
        let mut command = Command::new("tmux");
        command.arg("-S").arg(&self.socket_path).args(args);
        self.apply_env(&mut command);
        command.output().expect("run isolated tmux command")
    }

    fn run_aoe(&self, args: &[&str]) -> Output {
        let mut command = Command::new(&self.binary);
        command.args(args);
        self.apply_env(&mut command);
        command.output().expect("run aoe")
    }

    fn spawn_tui(&self) {
        let binary = self.binary.to_string_lossy();
        let project = self.project.to_string_lossy();
        let output = self.tmux(&[
            "new-session",
            "-d",
            "-s",
            &self.outer_session,
            "-x",
            "100",
            "-y",
            "30",
            "-c",
            &project,
            &binary,
        ]);
        assert!(
            output.status.success(),
            "spawn TUI: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn send_key(&self, key: &str) {
        let output = self.tmux(&["send-keys", "-t", &self.outer_session, key]);
        assert!(output.status.success(), "send key {key}");
    }

    fn type_text(&self, value: &str) {
        let output = self.tmux(&["send-keys", "-t", &self.outer_session, "-l", value]);
        assert!(output.status.success(), "type text");
    }

    fn capture(&self, target: &str) -> String {
        let output = self.tmux(&["capture-pane", "-t", target, "-p", "-S", "-200"]);
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn wait_for_screen(&self, target: &str, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if self.capture(target).contains(expected) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("missing {expected:?} in screen:\n{}", self.capture(target));
    }

    fn wait_for_file(&self, path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if path.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("timed out waiting for {}", path.display());
    }

    fn sessions_path(&self) -> PathBuf {
        self.config_dir().join("profiles/default/sessions.json")
    }

    fn sessions(&self) -> Vec<serde_json::Value> {
        let content = std::fs::read_to_string(self.sessions_path()).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    }

    fn add_codex_session(&self, title: &str) {
        let output = self.run_aoe(&[
            "add",
            self.project.to_str().unwrap(),
            "-t",
            title,
            "-c",
            "codex",
        ]);
        assert!(
            output.status.success(),
            "add session: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut sessions = self.sessions();
        let session = sessions
            .iter_mut()
            .find(|session| session["title"] == title)
            .expect("created session");
        session["cross_agent_team"] = serde_json::Value::Bool(true);
        std::fs::write(
            self.sessions_path(),
            serde_json::to_string_pretty(&sessions).unwrap(),
        )
        .expect("enable Cross Agent Team");
    }

    fn start_session(&self, title: &str) {
        let output = self.run_aoe(&["session", "start", title]);
        assert!(
            output.status.success(),
            "start session: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn session_name(&self, title: &str) -> String {
        let session = self
            .sessions()
            .into_iter()
            .find(|session| session["title"] == title)
            .expect("session record");
        let id = session["id"].as_str().expect("session id");
        format!("aoe_{}_{}", sanitize_title(title), &id[..id.len().min(8)])
    }

    fn wait_for_session_name(&self, title: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if self
                .sessions()
                .iter()
                .any(|session| session["title"] == title)
            {
                return self.session_name(title);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("session {title} was not created");
    }

    fn identity_key(&self, title: &str) -> String {
        self.sessions()
            .into_iter()
            .find(|session| session["title"] == title)
            .and_then(|session| session["xats_identity_key"].as_str().map(str::to_string))
            .filter(|key| !key.is_empty())
            .expect("persisted xats identity key")
    }

    fn pane_value(&self, target: &str, format: &str) -> String {
        let output = self.tmux(&["display-message", "-p", "-t", target, format]);
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn wait_for_pane_count(&self, session: &str, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if self.pane_value(session, "#{window_panes}") == expected {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("expected {expected} panes in {session}");
    }

    fn wait_for_pid_change(&self, target: &str, before: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            let current = self.pane_value(target, "#{pane_pid}");
            if !current.is_empty() && current != before {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("pane {target} was not respawned");
    }

    fn wait_for_recoverable(&self) {
        let deadline = Instant::now() + Duration::from_secs(40);
        while Instant::now() < deadline {
            if self.capture(&self.outer_session).contains("[recoverable]") {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        panic!("session did not become recoverable");
    }

    fn slot_count(&self, title: &str) -> usize {
        let id = self
            .sessions()
            .into_iter()
            .find(|session| session["title"] == title)
            .and_then(|session| session["id"].as_str().map(str::to_string))
            .expect("session id");
        let path = self.config_dir().join("profiles/default/aoe.db");
        let (store, _) =
            agent_of_empires::db::Store::open_with_schema_at(&path).expect("open isolated store");
        store
            .read_slots_for_instance(&id)
            .expect("read durable slots")
            .len()
    }

    fn wait_for_slot_count(&self, title: &str, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if self.slot_count(title) == expected {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("expected {expected} durable slots for {title}");
    }
}

impl Drop for CodexXatsHarness {
    fn drop(&mut self) {
        for session in self.sessions() {
            let Some(title) = session["title"].as_str() else {
                continue;
            };
            let Some(id) = session["id"].as_str() else {
                continue;
            };
            let target = format!("aoe_{}_{}", sanitize_title(title), &id[..id.len().min(8)]);
            let _ = self.tmux(&["kill-session", "-t", &target]);
        }
        let _ = self.tmux(&["kill-session", "-t", &self.outer_session]);
    }
}

fn current_uid() -> String {
    let output = Command::new("id").arg("-u").output().expect("run id -u");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn sanitize_title(title: &str) -> String {
    title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(20)
        .collect()
}

fn write_executable(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("write shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("set shim permissions");
    }
}

fn assert_codex_shell_panes(h: &CodexXatsHarness, session: &str, key: &str) {
    let shell_target = format!("{session}:.1");
    let shell_command = h.pane_value(&shell_target, "#{pane_start_command}");
    let codex_environment = std::fs::read_to_string(&h.codex_log).expect("Codex environment log");
    let expected_cwd = h.project.canonicalize().expect("canonical project");

    assert!(codex_environment.contains(&format!("XATS_IDENTITY_KEY={key}\n")));
    assert!(shell_command.contains("aoe-test-shell"));
    assert!(shell_command.contains(" -lc "));
    assert!(!shell_command.contains("bash -lc"));
    assert_eq!(
        h.pane_value(&shell_target, "#{pane_current_path}"),
        expected_cwd.to_string_lossy()
    );
}

fn select_dialog_tool(h: &CodexXatsHarness, field: &str, tool: &str) {
    let selected = format!("● {tool}");
    for _ in 0..32 {
        let screen = h.capture(&h.outer_session);
        let line = screen
            .lines()
            .find(|line| line.contains(field))
            .unwrap_or("");
        if line.contains(&selected) {
            return;
        }
        h.send_key("Right");
    }
    panic!("could not select {tool} in {field}");
}

fn create_codex_shell_session(h: &CodexXatsHarness, title: &str) -> String {
    h.send_key("n");
    h.wait_for_screen(&h.outer_session, "Cross Agent Team");
    h.type_text(title);
    h.send_key("Tab");

    for _ in 0..6 {
        h.send_key("Tab");
    }
    select_dialog_tool(h, "Right Pane Agent:", "shell");
    h.send_key("Enter");

    h.wait_for_session_name(title)
}

#[test]
#[serial]
fn test_new_codex_cross_agent_team_session_bootstraps_xats() {
    require_tmux!();
    let h = CodexXatsHarness::new("codex_xats_success");
    h.spawn_tui();
    h.wait_for_screen(&h.outer_session, "Agent of Empires");
    h.send_key("n");
    h.wait_for_screen(&h.outer_session, "Cross Agent Team");
    assert!(h
        .capture(&h.outer_session)
        .contains("Cross Agent Team: [x]"));

    h.type_text("codex-xats-e2e");
    h.send_key("Enter");
    h.wait_for_file(&h.codex_log);
    h.wait_for_file(&h.xats_log);

    let sessions = h.sessions();
    let session = sessions
        .iter()
        .find(|session| session["title"] == "codex-xats-e2e")
        .expect("created Codex session");
    assert_eq!(session["tool"], "codex");
    assert_eq!(session["cross_agent_team"], true);

    let xats_args = std::fs::read_to_string(&h.xats_log).unwrap();
    assert!(xats_args.contains("--no-install\ncross-agent-teams-mcp\n"));
    assert!(xats_args.contains("pre-register-codex-pane"));
    assert!(xats_args.contains(AGENT_ID));
    assert!(xats_args.contains("%"));

    let codex_args = std::fs::read_to_string(&h.codex_log).unwrap();
    let expected_project = h.project.canonicalize().expect("canonical project");
    assert!(codex_args.contains("--remote\nws://127.0.0.1:8799\n"));
    assert!(
        codex_args.contains(&format!("-C\n{}\n", expected_project.display())),
        "unexpected Codex args: {codex_args:?}"
    );
    assert!(codex_args.contains(&format!("xats.agent_id=\"{AGENT_ID}\"")));
    assert!(!codex_args.contains("--dangerously-bypass-approvals-and-sandbox"));
}

#[test]
#[serial]
fn test_codex_xats_pre_registration_failure_is_visible() {
    require_tmux!();
    let h = CodexXatsHarness::new("codex_xats_prereg_failure");
    h.add_codex_session("codex-xats-prereg-failure");
    std::fs::write(&h.prereg_failure, "fail").expect("enable prereg failure");

    h.start_session("codex-xats-prereg-failure");
    let session_name = h.session_name("codex-xats-prereg-failure");
    h.wait_for_screen(&session_name, "controlled preregistration failure");

    assert!(h.xats_log.exists());
    assert!(
        !h.codex_log.exists(),
        "Codex must not launch after xats failure"
    );
}

#[test]
#[serial]
fn test_codex_xats_app_server_failure_is_visible() {
    require_tmux!();
    let h = CodexXatsHarness::new("codex_xats_app_server_failure");
    h.add_codex_session("codex-xats-app-server-failure");
    std::fs::write(&h.app_server_failure, "fail").expect("disable app server");

    h.start_session("codex-xats-app-server-failure");
    let session_name = h.session_name("codex-xats-app-server-failure");
    h.wait_for_screen(
        &session_name,
        "[xats] Codex app-server is not listening on ws://127.0.0.1:8799.",
    );

    assert!(!h.xats_log.exists(), "pre-registration must not run");
    assert!(
        !h.codex_log.exists(),
        "Codex must not launch without app-server"
    );
}

#[test]
#[serial]
fn test_codex_and_same_cwd_shell_restart_and_recover_together() {
    require_tmux!();
    let h = CodexXatsHarness::new("codex_shell_lifecycle");
    write_executable(
        &h.bin_dir.path().join("codex"),
        "#!/bin/sh\nprintf 'XATS_IDENTITY_KEY=%s\\n' \"${XATS_IDENTITY_KEY-}\" > \"$CODEX_ARGS_LOG\"\nprintf '%s\\n' \"$@\" >> \"$CODEX_ARGS_LOG\"\nexec sleep 2147483647\n",
    );
    let title = "codex-shell-lifecycle";
    h.spawn_tui();
    h.wait_for_screen(&h.outer_session, "Agent of Empires");
    let session = create_codex_shell_session(&h, title);
    h.wait_for_pane_count(&session, "2");
    h.wait_for_slot_count(title, 2);
    h.wait_for_file(&h.codex_log);

    let key = h.identity_key(title);
    assert_codex_shell_panes(&h, &session, &key);
    let codex_pid = h.pane_value(&format!("{session}:.0"), "#{pane_pid}");
    let shell_pid = h.pane_value(&format!("{session}:.1"), "#{pane_pid}");

    std::fs::remove_file(&h.codex_log).expect("clear Codex environment log");
    h.send_key("C");
    h.wait_for_pid_change(&format!("{session}:.0"), &codex_pid);
    h.wait_for_pid_change(&format!("{session}:.1"), &shell_pid);
    h.wait_for_file(&h.codex_log);
    assert_eq!(h.identity_key(title), key);
    assert_codex_shell_panes(&h, &session, &key);

    let killed = h.tmux(&["kill-session", "-t", &session]);
    assert!(killed.status.success(), "kill managed session: {killed:?}");
    std::fs::remove_file(&h.codex_log).expect("clear Codex environment log");
    h.wait_for_recoverable();
    h.send_key("C");
    h.wait_for_pane_count(&session, "2");
    h.wait_for_file(&h.codex_log);
    assert_eq!(h.identity_key(title), key);
    assert_codex_shell_panes(&h, &session, &key);
}

#[test]
#[serial]
fn test_percent_shell_is_managed_but_raw_split_is_not() {
    require_tmux!();
    let h = CodexXatsHarness::new("percent_shell_slot");
    let title = "percent-shell-slot";
    h.add_codex_session(title);
    h.start_session(title);
    let session = h.session_name(title);

    h.spawn_tui();
    h.wait_for_screen(&h.outer_session, title);
    h.send_key("%");
    h.wait_for_screen(&h.outer_session, "Add Agent Pane");
    select_dialog_tool(&h, "Agent:", "shell");
    h.send_key("Enter");

    h.wait_for_pane_count(&session, "2");
    h.wait_for_slot_count(title, 2);

    let project = h.project.to_string_lossy();
    let split = h.tmux(&["split-window", "-h", "-t", &session, "-c", &project]);
    assert!(split.status.success(), "create raw split: {split:?}");
    h.wait_for_pane_count(&session, "3");
    std::thread::sleep(Duration::from_millis(1_600));
    assert_eq!(h.slot_count(title), 2, "raw split must stay slotless");
}
