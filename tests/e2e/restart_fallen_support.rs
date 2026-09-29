use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::claude_model_support::{add_and_start, db_path, run_record_pane, wait_for_count};
use crate::harness::TuiTestHarness;

pub(super) struct Fixture {
    pub(super) h: TuiTestHarness,
    pub(super) native: PathBuf,
    pub(super) gate: PathBuf,
    pub(super) session: String,
    pub(super) panes: Vec<String>,
}

pub(super) fn tmux(h: &TuiTestHarness, args: &[&str]) -> String {
    let output = Command::new("tmux")
        .arg("-S")
        .arg(h.tmux_socket_path())
        .args(args)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .expect("run private tmux");
    assert!(
        output.status.success(),
        "{:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub(super) fn evidence(name: &str, body: &str) {
    match std::env::var("AOE_ACCEPTANCE_EVIDENCE") {
        Ok(root) => std::fs::write(PathBuf::from(root).join(name), body).expect("write evidence"),
        Err(std::env::VarError::NotPresent) => {}
        Err(error) => panic!("invalid evidence directory: {error}"),
    }
}

pub(super) fn wait_until(h: &TuiTestHarness, label: &str, mut predicate: impl FnMut() -> bool) {
    let start = Instant::now();
    while !predicate() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "{label}\n{}",
            h.capture_screen()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn install_gate(h: &TuiTestHarness) -> PathBuf {
    let gate = h.home_path().join("poll-gate");
    std::fs::create_dir(&gate).expect("create gate");
    let real_tmux = Command::new("which").arg("tmux").output().unwrap();
    assert!(real_tmux.status.success());
    let real_tmux = String::from_utf8(real_tmux.stdout).unwrap();
    let wrapper = include_str!("restart_fallen_tmux.py")
        .replace("__GATE__", &serde_json::to_string(&gate).unwrap())
        .replace(
            "__SOCKET__",
            &serde_json::to_string(h.tmux_socket_path()).unwrap(),
        )
        .replace(
            "__BINARY__",
            &serde_json::to_string(h.binary_path()).unwrap(),
        )
        .replace(
            "__TMUX__",
            &serde_json::to_string(real_tmux.trim()).unwrap(),
        );
    h.install_stub_script("tmux", &wrapper);
    gate
}

fn record_panes(h: &TuiTestHarness, id: &str, panes: &[String], native: &std::path::Path) {
    let cwd = h.project_path().to_str().unwrap().to_string();
    let tokens = [
        "019d1af9-a899-7df1-8f7d-a244126e5ded",
        "019d1af9-a899-7df1-8f7d-a244126e5dee",
    ];
    for (pane, token) in panes.iter().zip(tokens) {
        tmux(
            h,
            &["respawn-pane", "-k", "-t", pane, native.to_str().unwrap()],
        );
        assert!(run_record_pane(h, pane, id, "codex", token, &cwd));
    }
}

fn new_fixture(scope: &str) -> (Fixture, String) {
    let h = TuiTestHarness::new(scope);
    let title = "Dual Restart Acceptance";
    let id = add_and_start(&h, title, "codex", None);
    let native = h.install_native_stub("codex").expect("compile codex stub");
    let session = agent_of_empires::tmux::Session::generate_name(&id, title);
    let panes = vec![
        h.tmux_display_message(&session, "#{pane_id}"),
        h.split_window_get_pane(&session),
    ];
    record_panes(&h, &id, &panes, &native);
    let gate = install_gate(&h);
    (
        Fixture {
            h,
            native,
            gate,
            session,
            panes,
        },
        id,
    )
}

pub(super) fn setup_fallen(scope: &str) -> Fixture {
    let (mut f, id) = new_fixture(scope);
    wait_until(&f.h, "both native stubs live", || f.live());
    f.exit_both();
    wait_until(&f.h, "both panes fell before TUI sampling", || {
        f.panes.iter().all(|pane| {
            let command = tmux(
                &f.h,
                &[
                    "display-message",
                    "-p",
                    "-t",
                    pane,
                    "#{pane_current_command}:#{pane_dead}",
                ],
            );
            ["zsh:0", "bash:0", "sh:0"].contains(&command.as_str())
        })
    });
    f.h.spawn_tui_without_tmux_env();
    f.h.wait_for("Dual Restart Acceptance");
    f.h.resize_window(f.h.session_name(), 220, 60);
    wait_for_count(
        &f.h,
        &db_path(&f.h),
        &format!("SELECT count(*) FROM agent_slot WHERE instance_id='{id}';"),
        "2",
    );
    f.h.wait_for_timeout("dropped to shell", Duration::from_secs(20));
    for pane in &f.panes {
        f.h.assert_screen_contains(pane);
    }
    save_environment(&f, scope);
    f
}

fn save_environment(f: &Fixture, scope: &str) {
    evidence(
        &format!("{scope}-environment.txt"),
        &format!(
            "socket={}\nhome={}\nsessions={},{}\npanes={:?}\nserver_pid={}\n",
            f.h.tmux_socket_path().display(),
            f.h.home_path().display(),
            f.h.session_name(),
            f.session,
            f.panes,
            tmux(&f.h, &["display-message", "-p", "#{pid}"])
        ),
    );
    evidence(
        &format!("{scope}-initial-fallen.txt"),
        &f.h.capture_screen(),
    );
}

impl Fixture {
    pub(super) fn live(&self) -> bool {
        self.panes.iter().all(|pane| {
            tmux(
                &self.h,
                &[
                    "display-message",
                    "-p",
                    "-t",
                    pane,
                    "#{pane_current_command}:#{pane_dead}",
                ],
            ) == "codex:0"
        })
    }

    pub(super) fn exit_both(&self) {
        for pane in &self.panes {
            self.h.send_keys_to_target(pane, "C-c");
        }
    }

    pub(super) fn healthy_for(&self, seconds: u64) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(seconds) {
            let screen = self.h.capture_screen();
            assert!(!screen.contains("dropped to shell"), "{screen}");
            assert!(!screen.contains("Error:"), "{screen}");
            assert!(
                !screen.lines().any(|line| line
                    .split_once("Status:")
                    .is_some_and(|(_, state)| state.trim_start().starts_with("Error"))),
                "{screen}"
            );
            assert!(self.live(), "agent exited during health check");
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    pub(super) fn arm(&self) {
        std::fs::write(self.gate.join("arm"), self.panes.join("\n")).unwrap();
    }

    pub(super) fn await_sample(&self, scope: &str) {
        wait_until(&self.h, "poll output captured", || {
            self.gate.join("blocked").exists()
        });
        let parent = std::fs::read_to_string(self.gate.join("parent.txt")).unwrap();
        assert_eq!(parent.trim(), self.h.binary_path().to_str().unwrap());
        evidence(&format!("{scope}-parent.txt"), &parent);
        evidence(
            &format!("{scope}-sample.txt"),
            &std::fs::read_to_string(self.gate.join("sample.txt")).unwrap(),
        );
    }

    pub(super) fn release(&self) {
        std::fs::write(self.gate.join("release"), "release").unwrap();
        wait_until(&self.h, "poll output released", || {
            self.gate.join("released").exists()
        });
        assert!(!self.gate.join("timeout").exists(), "gate timed out");
    }

    pub(super) fn finish(self, scope: &str) {
        let socket = self.h.tmux_socket_path().to_owned();
        let home = self.h.home_path().to_owned();
        drop(self);
        assert!(!socket.exists(), "private socket remains");
        assert!(!home.exists(), "isolated HOME remains");
        evidence(
            &format!("{scope}-cleanup.txt"),
            "private socket absent\nisolated HOME absent\n",
        );
    }
}
