//! Core e2e test harness built on tmux.
//!
//! `TuiTestHarness` launches `aoe` in a detached tmux session with an isolated
//! `$HOME`, sends keystrokes, captures screen output, and polls for expected
//! text. It also provides `run_cli` for exercising CLI subcommands as plain
//! subprocesses (no tmux).
//!
//! ## Recording
//!
//! Set `RECORD_E2E=1` to record each TUI test as an asciinema `.cast` file and
//! convert it to a GIF via `agg`. Recordings are saved to
//! `target/e2e-recordings/`. Both `asciinema` and `agg` must be on `$PATH`.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

// ---------------------------------------------------------------------------
// tmux availability guard
// ---------------------------------------------------------------------------

pub fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Skip the calling test if tmux is not installed.
macro_rules! require_tmux {
    () => {
        if !$crate::harness::tmux_available() {
            eprintln!("Skipping test: tmux not available");
            return;
        }
    };
}
pub(crate) use require_tmux;

// ---------------------------------------------------------------------------
// Recording helpers
// ---------------------------------------------------------------------------

fn recording_enabled() -> bool {
    std::env::var("RECORD_E2E").is_ok_and(|v| v == "1" || v == "true")
}

fn asciinema_available() -> bool {
    Command::new("asciinema")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn agg_available() -> bool {
    Command::new("agg")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn recordings_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/e2e-recordings");
    std::fs::create_dir_all(&dir).expect("create recordings dir");
    dir
}

fn convert_cast_to_gif(cast_path: &Path) {
    if !agg_available() {
        eprintln!(
            "agg not found -- skipping GIF conversion for {}",
            cast_path.display()
        );
        return;
    }

    let gif_path = cast_path.with_extension("gif");
    let status = Command::new("agg")
        .args(["--font-size", "14"])
        .arg(cast_path)
        .arg(&gif_path)
        .status();

    match status {
        Ok(s) if s.success() => {
            eprintln!("Recorded GIF: {}", gif_path.display());
        }
        Ok(s) => {
            eprintln!("agg exited with {}, GIF not created", s);
        }
        Err(e) => {
            eprintln!("agg failed: {}", e);
        }
    }
}

// ---------------------------------------------------------------------------
// TuiTestHarness
// ---------------------------------------------------------------------------

pub struct TuiTestHarness {
    session_name: String,
    test_name: String,
    home_dir: TempDir,
    _stub_dir: TempDir,
    binary_path: PathBuf,
    stub_path: PathBuf,
    socket_path: PathBuf,
    spawned: bool,
    control_client: Option<Child>,
    recording: bool,
    cast_path: Option<PathBuf>,
}

#[allow(dead_code)]
impl TuiTestHarness {
    /// Create a new harness with an isolated `$HOME` and a fake `claude` stub
    /// so tool detection succeeds.
    pub fn new(test_name: &str) -> Self {
        let home_dir = TempDir::new().expect("failed to create temp home");
        let stub_dir = TempDir::new().expect("failed to create stub dir");

        // Unique session name to avoid collisions.
        let session_name = format!("aoe_e2e_{}_{}", test_name, std::process::id());

        // Path to unique tmux socket for this test.
        let socket_path = home_dir.path().join("tmux.sock");

        // Create a fake `claude` script so `which claude` succeeds.
        let stub_path = stub_dir.path().to_path_buf();
        let claude_stub = stub_path.join("claude");
        std::fs::write(&claude_stub, "#!/bin/sh\nexit 0\n").expect("write claude stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&claude_stub, std::fs::Permissions::from_mode(0o755))
                .expect("chmod claude stub");
        }

        // Pre-seed config.toml to skip the welcome dialog and update checks.
        // On Linux the app uses $XDG_CONFIG_HOME/agent-of-empires/ (set below),
        // on macOS it uses $HOME/.agent-of-empires/.
        let config_dir = if cfg!(target_os = "linux") {
            home_dir.path().join(".config").join("agent-of-empires")
        } else {
            home_dir.path().join(".agent-of-empires")
        };
        std::fs::create_dir_all(&config_dir).expect("create config dir");
        let config_content = format!(
            r#"[updates]
check_enabled = false

[app_state]
has_seen_welcome = true
last_seen_version = "{}"
"#,
            env!("CARGO_PKG_VERSION")
        );
        std::fs::write(config_dir.join("config.toml"), config_content).expect("write config.toml");

        // Create default profile directory.
        std::fs::create_dir_all(config_dir.join("profiles").join("default"))
            .expect("create default profile dir");

        let binary_path = PathBuf::from(env!("CARGO_BIN_EXE_aoe"));

        let recording = recording_enabled() && asciinema_available();
        if recording_enabled() && !asciinema_available() {
            eprintln!("RECORD_E2E is set but asciinema is not installed -- recording disabled");
        }

        Self {
            session_name,
            test_name: test_name.to_string(),
            home_dir,
            _stub_dir: stub_dir,
            binary_path,
            stub_path,
            socket_path,
            spawned: false,
            control_client: None,
            recording,
            cast_path: None,
        }
    }

    /// Build the PATH with the stub directory prepended so fake `claude` is found.
    fn env_path(&self) -> String {
        let system_path = std::env::var("PATH").unwrap_or_default();
        format!("{}:{}", self.stub_path.display(), system_path)
    }

    /// Build the shell command string to run inside the tmux session.
    /// When recording, wraps the command with `asciinema rec`.
    fn build_tmux_command(&mut self, args: &[&str]) -> String {
        let mut aoe_cmd = self.binary_path.display().to_string();
        for arg in args {
            aoe_cmd.push(' ');
            aoe_cmd.push_str(arg);
        }

        if self.recording {
            let cast_path = recordings_dir().join(format!("{}.cast", self.test_name));
            let cmd = format!(
                "asciinema rec --overwrite --cols 100 --rows 30 -c '{}' {}",
                aoe_cmd,
                cast_path.display()
            );
            self.cast_path = Some(cast_path);
            cmd
        } else {
            aoe_cmd
        }
    }

    /// Spawn `aoe` (no arguments = TUI mode) inside a detached tmux session
    /// with a fixed 100x30 terminal.
    pub fn spawn_tui(&mut self) {
        self.spawn(&[]);
    }

    /// Spawn `aoe <args>` inside a detached tmux session.
    pub fn spawn(&mut self, args: &[&str]) {
        let cmd_str = self.build_tmux_command(args);

        let output = Command::new("tmux")
            .arg("-S")
            .arg(&self.socket_path)
            .arg("new-session")
            .arg("-d")
            .arg("-s")
            .arg(&self.session_name)
            .arg("-x")
            .arg("100")
            .arg("-y")
            .arg("30")
            .arg(&cmd_str)
            .env("HOME", self.home_dir.path())
            .env("XDG_CONFIG_HOME", self.home_dir.path().join(".config"))
            .env("PATH", self.env_path())
            .env("TERM", "xterm-256color")
            .env("AGENT_OF_EMPIRES_PROFILE", "default")
            .output()
            .expect("failed to run tmux new-session");

        assert!(
            output.status.success(),
            "tmux new-session failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        self.spawned = true;

        // Brief pause for the process to initialize.
        // Recording adds overhead so wait a bit longer.
        let delay = if self.recording { 500 } else { 300 };
        std::thread::sleep(Duration::from_millis(delay));
    }

    pub fn attach_control_client(&mut self) {
        assert!(self.spawned, "must call spawn_tui() or spawn() first");
        if self.control_client.is_some() {
            return;
        }

        let mut command = if cfg!(target_os = "macos") {
            let mut command = Command::new("script");
            command
                .arg("-q")
                .arg("/dev/null")
                .arg("tmux")
                .arg("-S")
                .arg(&self.socket_path)
                .arg("attach-session")
                .arg("-t")
                .arg(&self.session_name);
            command
        } else {
            let mut command = Command::new("script");
            command
                .arg("-q")
                .arg("-c")
                .arg(format!(
                    "tmux -S '{}' attach-session -t '{}'",
                    self.socket_path.display(),
                    self.session_name
                ))
                .arg("/dev/null");
            command
        };

        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to attach control-mode tmux client");

        self.control_client = Some(child);

        let start = Instant::now();
        while start.elapsed() <= Duration::from_secs(10) {
            if self
                .tmux_command(&["list-clients", "-F", "#{client_name}"])
                .ok()
                .is_some_and(|output| output.status.success() && !output.stdout.is_empty())
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        panic!("Timed out waiting for control-mode tmux client to attach");
    }

    pub fn attach_client_to_session(&self, target_session: &str) -> Child {
        assert!(self.spawned, "must call spawn_tui() or spawn() first");

        let mut command = if cfg!(target_os = "macos") {
            let mut command = Command::new("script");
            command
                .arg("-q")
                .arg("/dev/null")
                .arg("tmux")
                .arg("-S")
                .arg(&self.socket_path)
                .arg("attach-session")
                .arg("-t")
                .arg(target_session);
            command
        } else {
            let mut command = Command::new("script");
            command
                .arg("-q")
                .arg("-c")
                .arg(format!(
                    "tmux -S '{}' attach-session -t '{}'",
                    self.socket_path.display(),
                    target_session
                ))
                .arg("/dev/null");
            command
        };

        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to attach tmux client")
    }

    pub fn spawn_cli_attach_process(&self, identifier: &str, tmux_env: Option<&str>) -> Child {
        self.ensure_tmux_server();

        let mut command = if cfg!(target_os = "macos") {
            let mut command = Command::new("script");
            command
                .arg("-q")
                .arg("/dev/null")
                .arg(&self.binary_path)
                .arg("session")
                .arg("attach")
                .arg(identifier);
            command
        } else {
            let mut command = Command::new("script");
            command
                .arg("-q")
                .arg("-c")
                .arg(format!(
                    "'{}' session attach '{}'",
                    self.binary_path.display(),
                    identifier
                ))
                .arg("/dev/null");
            command
        };

        command
            .env("HOME", self.home_dir.path())
            .env("XDG_CONFIG_HOME", self.home_dir.path().join(".config"))
            .env("PATH", self.env_path())
            .env("TERM", "xterm-256color")
            .env("AGENT_OF_EMPIRES_PROFILE", "default");

        if let Some(tmux_env) = tmux_env {
            command.env("TMUX", tmux_env);
        } else {
            command.env_remove("TMUX");
        }

        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn aoe session attach process")
    }

    /// Send one or more tmux key names (e.g. "Enter", "Escape", "q", "C-c").
    pub fn send_keys(&self, keys: &str) {
        assert!(self.spawned, "must call spawn_tui() or spawn() first");
        let output = self
            .tmux_command(&["send-keys", "-t", &self.session_name, keys])
            .expect("failed to send keys");
        assert!(
            output.status.success(),
            "send-keys failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        // Let the TUI process the keystroke.
        std::thread::sleep(Duration::from_millis(50));
    }

    /// Send literal text (prevents "Enter" in text from being interpreted as
    /// the Enter key).
    pub fn type_text(&self, text: &str) {
        assert!(self.spawned, "must call spawn_tui() or spawn() first");
        let output = self
            .tmux_command(&["send-keys", "-t", &self.session_name, "-l", text])
            .expect("failed to type text");
        assert!(
            output.status.success(),
            "type_text failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    /// Capture the current screen contents as plain text (no ANSI escapes).
    pub fn capture_screen(&self) -> String {
        assert!(self.spawned, "must call spawn_tui() or spawn() first");
        let output = self
            .tmux_command(&["capture-pane", "-t", &self.session_name, "-p"])
            .expect("failed to capture pane");
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    /// Poll `capture_screen()` until `text` appears. Panics with a screen dump
    /// if the default timeout (10s) is exceeded.
    pub fn wait_for(&self, text: &str) {
        self.wait_for_timeout(text, Duration::from_secs(10));
    }

    /// Like `wait_for` but with a custom timeout.
    pub fn wait_for_timeout(&self, text: &str, timeout: Duration) {
        let start = Instant::now();
        loop {
            let screen = self.capture_screen();
            if screen.contains(text) {
                return;
            }
            if start.elapsed() > timeout {
                panic!(
                    "Timed out waiting for {:?} after {:?}.\n\n--- Screen capture ---\n{}\n--- End screen capture ---",
                    text, timeout, screen
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Poll until `text` disappears from the screen.
    pub fn wait_for_absent(&self, text: &str, timeout: Duration) {
        let start = Instant::now();
        loop {
            let screen = self.capture_screen();
            if !screen.contains(text) {
                return;
            }
            if start.elapsed() > timeout {
                panic!(
                    "Timed out waiting for {:?} to disappear after {:?}.\n\n--- Screen capture ---\n{}\n--- End screen capture ---",
                    text, timeout, screen
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Assert that the screen currently contains `text`.
    pub fn assert_screen_contains(&self, text: &str) {
        let screen = self.capture_screen();
        assert!(
            screen.contains(text),
            "Expected screen to contain {:?}.\n\n--- Screen capture ---\n{}\n--- End screen capture ---",
            text, screen
        );
    }

    /// Assert that the screen does NOT contain `text`.
    pub fn assert_screen_not_contains(&self, text: &str) {
        let screen = self.capture_screen();
        assert!(
            !screen.contains(text),
            "Expected screen NOT to contain {:?}.\n\n--- Screen capture ---\n{}\n--- End screen capture ---",
            text, screen
        );
    }

    /// Run `aoe <args>` as a subprocess (not in tmux) with the same env
    /// isolation. Returns the `Output` (stdout, stderr, status).
    pub fn run_cli(&self, args: &[&str]) -> Output {
        Command::new(&self.binary_path)
            .args(args)
            .env("HOME", self.home_dir.path())
            .env("XDG_CONFIG_HOME", self.home_dir.path().join(".config"))
            .env("PATH", self.env_path())
            .env("AGENT_OF_EMPIRES_PROFILE", "default")
            .output()
            .expect("failed to run aoe CLI")
    }

    /// Run `aoe <args>` as a subprocess while targeting this harness's tmux
    /// socket via the `TMUX` environment variable.
    pub fn run_cli_in_tmux(&self, args: &[&str]) -> Output {
        self.ensure_tmux_server();

        Command::new(&self.binary_path)
            .args(args)
            .env("HOME", self.home_dir.path())
            .env("XDG_CONFIG_HOME", self.home_dir.path().join(".config"))
            .env("PATH", self.env_path())
            .env("AGENT_OF_EMPIRES_PROFILE", "default")
            .env("TMUX", format!("{},1,0", self.socket_path.display()))
            .output()
            .expect("failed to run aoe CLI inside tmux")
    }

    pub fn run_cli_with_tmux_env(&self, tmux_env: &str, args: &[&str]) -> Output {
        self.ensure_tmux_server();

        Command::new(&self.binary_path)
            .args(args)
            .env("HOME", self.home_dir.path())
            .env("XDG_CONFIG_HOME", self.home_dir.path().join(".config"))
            .env("PATH", self.env_path())
            .env("AGENT_OF_EMPIRES_PROFILE", "default")
            .env("TMUX", tmux_env)
            .output()
            .expect("failed to run aoe CLI with custom TMUX env")
    }

    /// Path to the isolated home directory for custom test setup.
    pub fn home_path(&self) -> &Path {
        self.home_dir.path()
    }

    pub fn binary_path(&self) -> &Path {
        &self.binary_path
    }

    pub fn tmux_socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Create and return a test project directory inside the temp home.
    pub fn project_path(&self) -> PathBuf {
        let p = self.home_dir.path().join("test-project");
        std::fs::create_dir_all(&p).expect("create project dir");
        p
    }

    pub fn session_name(&self) -> &str {
        &self.session_name
    }

    pub fn tmux_show_option(&self, target: &str, option: &str) -> String {
        let output = self
            .tmux_command(&["show-options", "-t", target, "-v", option])
            .expect("failed to show tmux option");
        assert!(
            output.status.success(),
            "show-options failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    pub fn tmux_show_global_option(&self, option: &str) -> String {
        let output = self
            .tmux_command(&["show-options", "-g", "-v", option])
            .expect("failed to show global tmux option");
        assert!(
            output.status.success(),
            "show global option failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    pub fn tmux_set_global_option(&self, option: &str, value: &str) {
        let output = self
            .tmux_command(&["set-option", "-gq", option, value])
            .expect("failed to set global tmux option");
        assert!(
            output.status.success(),
            "set global option failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    pub fn tmux_single_client_name(&self) -> String {
        let clients = self.tmux_client_names();
        assert_eq!(
            clients.len(),
            1,
            "expected exactly one tmux client, got {:?}",
            clients
        );
        clients[0].clone()
    }

    pub fn tmux_client_names(&self) -> Vec<String> {
        let output = self
            .tmux_command(&["list-clients", "-F", "#{client_name}"])
            .expect("failed to list tmux clients");
        assert!(
            output.status.success(),
            "list-clients failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_string)
            .collect()
    }

    pub fn wait_for_client_count(&self, expected_count: usize) {
        let start = Instant::now();
        while start.elapsed() <= Duration::from_secs(10) {
            if self.tmux_client_names().len() == expected_count {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        panic!(
            "Timed out waiting for {} tmux client(s); found {:?}",
            expected_count,
            self.tmux_client_names()
        );
    }

    pub fn tmux_client_session(&self, client_name: &str) -> String {
        let output = self
            .tmux_command(&["list-clients", "-F", "#{client_name}\t#{session_name}"])
            .expect("failed to list tmux clients with sessions");
        assert!(
            output.status.success(),
            "list-clients with sessions failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout
            .lines()
            .find_map(|line| {
                let (listed_client, session_name) = line.split_once('\t')?;
                (listed_client == client_name).then(|| session_name.to_string())
            })
            .unwrap_or_else(|| panic!("no tmux client session found for {}", client_name))
    }

    pub fn wait_for_client_session(&self, client_name: &str, expected_session: &str) {
        let start = Instant::now();
        while start.elapsed() <= Duration::from_secs(10) {
            if self.tmux_client_session(client_name) == expected_session {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        panic!(
            "Timed out waiting for client {} to switch to session {}",
            client_name, expected_session
        );
    }

    pub fn tmux_switch_client(&self, client_name: &str, target_session: &str) {
        let output = self
            .tmux_command(&["switch-client", "-c", client_name, "-t", target_session])
            .expect("failed to switch tmux client");
        assert!(
            output.status.success(),
            "switch-client failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    pub fn tmux_show_window_option(&self, target: &str, option: &str) -> String {
        let output = self
            .tmux_command(&["show-window-options", "-t", target, "-v", option])
            .expect("failed to show tmux window option");
        assert!(
            output.status.success(),
            "show-window-options failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    pub fn tmux_display_message(&self, target: &str, format: &str) -> String {
        let output = self
            .tmux_command(&["display-message", "-t", target, "-p", format])
            .expect("failed to run tmux display-message");
        assert!(
            output.status.success(),
            "display-message failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    pub fn current_client_session(&self) -> String {
        let output = self
            .tmux_command(&["list-clients", "-F", "#{session_name}"])
            .expect("failed to list tmux clients");
        assert!(
            output.status.success(),
            "list-clients failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    pub fn send_keys_to_target(&self, target: &str, keys: &str) {
        let output = self
            .tmux_command(&["send-keys", "-t", target, keys])
            .expect("failed to send keys to target");
        assert!(
            output.status.success(),
            "send-keys target failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    pub fn send_keys_to_client(&self, client_name: &str, keys: &str) {
        let output = self
            .tmux_command(&["send-keys", "-K", "-c", client_name, keys])
            .expect("failed to send keys to client");
        assert!(
            output.status.success(),
            "send-keys client failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    pub fn type_text_to_target(&self, target: &str, text: &str) {
        let output = self
            .tmux_command(&["send-keys", "-t", target, "-l", text])
            .expect("failed to type text to target");
        assert!(
            output.status.success(),
            "type-text target failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    pub fn kill_tmux_target(&self, target: &str) {
        let output = self
            .tmux_command(&["kill-session", "-t", target])
            .expect("failed to kill tmux target");
        assert!(
            output.status.success(),
            "kill-session target failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    pub fn create_detached_shell_session(&self, session_name: &str) {
        self.ensure_tmux_server();
        let output = self
            .tmux_command(&["new-session", "-d", "-s", session_name])
            .expect("failed to create detached tmux session");
        assert!(
            output.status.success(),
            "new-session failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    pub fn tmux_list_key(&self, table: &str, key: &str) -> Output {
        self.tmux_command(&["list-keys", "-T", table, key])
            .expect("failed to list tmux key binding")
    }

    /// Check whether the tmux session is still alive.
    pub fn session_alive(&self) -> bool {
        self.tmux_command(&["has-session", "-t", &self.session_name])
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Wait until the tmux session terminates (the process exits).
    pub fn wait_for_exit(&self, timeout: Duration) {
        let start = Instant::now();
        loop {
            if !self.session_alive() {
                return;
            }
            if start.elapsed() > timeout {
                panic!(
                    "Timed out waiting for session {} to exit after {:?}",
                    self.session_name, timeout
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn kill_session(&self) {
        let _ = self.tmux_command(&["kill-session", "-t", &self.session_name]);
    }

    fn ensure_tmux_server(&self) {
        let output = self
            .tmux_command(&["start-server"])
            .expect("failed to start tmux server");
        assert!(
            output.status.success(),
            "start-server failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn tmux_command(&self, args: &[&str]) -> std::io::Result<Output> {
        let mut cmd = Command::new("tmux");
        cmd.arg("-S").arg(&self.socket_path);
        cmd.args(args);
        cmd.output()
    }
}

impl Drop for TuiTestHarness {
    fn drop(&mut self) {
        if let Some(mut client) = self.control_client.take() {
            let _ = client.kill();
            let _ = client.wait();
        }

        if self.spawned {
            self.kill_session();
        }

        // Sessions spawned by `aoe add --launch` (e.g. fork tests) land on the
        // default tmux socket, not the harness socket, so kill_session() misses
        // them. Sweep any session whose pane still lives inside this test's
        // isolated HOME so orphan `claude` children don't survive the test.
        kill_inner_aoe_sessions(self.home_dir.path());

        // Convert recording to GIF if one was produced.
        if let Some(cast_path) = &self.cast_path {
            // Give asciinema a moment to finalize the file after the session ends.
            std::thread::sleep(Duration::from_millis(200));
            if cast_path.exists() {
                convert_cast_to_gif(cast_path);
            }
        }
    }
}

fn kill_inner_aoe_sessions(home_root: &Path) {
    let Ok(root) = home_root.canonicalize() else {
        return;
    };

    let Ok(output) = Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{session_name}\t#{pane_current_path}",
        ])
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }

    let listing = String::from_utf8_lossy(&output.stdout);
    let mut killed = std::collections::HashSet::new();
    for line in listing.lines() {
        let Some((session, path)) = line.split_once('\t') else {
            continue;
        };
        let Ok(path) = Path::new(path).canonicalize() else {
            continue;
        };
        if !path.starts_with(&root) {
            continue;
        }
        if killed.insert(session.to_string()) {
            let _ = Command::new("tmux")
                .args(["kill-session", "-t", session])
                .output();
        }
    }
}
