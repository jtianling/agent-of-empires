//! Regression guard for the `tmux-test-isolation` / `tmux-command-seam`
//! capability.
//!
//! All AoE tmux invocations MUST go through the single `tmux_command()` seam in
//! `src/tmux/mod.rs`, which applies `-L <socket-name>` (and forces a private
//! socket under test). A bare `Command::new("tmux")` anywhere else bypasses the
//! seam and could hit the developer's default/live socket. Integration tests
//! (separate crate, where the seam's test path is inactive) MUST instead build
//! their tmux commands with an explicit `-L` private socket.
//!
//! See AGENTS.md "Tmux Session Safety".

use std::fs;
use std::path::{Path, PathBuf};

/// The ONLY production site allowed to construct a bare `tmux` command: the seam
/// definition itself.
const SEAM_FILE: &str = "src/tmux/mod.rs";

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // e2e tests are isolated by the harness (its own `-S <socket>`); skip.
        if path.ends_with("e2e") {
            continue;
        }
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// No `Command::new("tmux")` outside the seam: everything routes through
/// `tmux_command()` so socket selection is centralized and cannot leak.
#[test]
fn no_bare_tmux_command_outside_seam() {
    let mut files = Vec::new();
    collect_rs_files(Path::new("src"), &mut files);

    let mut offenders = Vec::new();
    for path in files {
        let is_seam = path
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with(SEAM_FILE);
        let Ok(src) = fs::read_to_string(&path) else {
            continue;
        };
        let count = src.matches("Command::new(\"tmux\")").count();
        // The seam file is allowed exactly one (the `tmux_command()` builder).
        let allowed = if is_seam { 1 } else { 0 };
        if count > allowed {
            offenders.push(format!(
                "{} ({} bare tmux command(s))",
                path.display(),
                count
            ));
        }
    }

    assert!(
        offenders.is_empty(),
        "Bare `Command::new(\"tmux\")` outside the `tmux_command()` seam -- route \
         it through `crate::tmux::tmux_command()` (see AGENTS.md \"Tmux Session \
         Safety\"): {offenders:?}"
    );
}

/// Integration tests (`tests/*.rs`) compile the lib without `#[cfg(test)]`, so
/// the seam's private-socket safety net is inactive there. Any such test that
/// builds a destructive `tmux` command MUST pin an explicit `-L` private socket.
#[test]
fn integration_tests_pin_private_socket() {
    let mut files = Vec::new();
    collect_rs_files(Path::new("tests"), &mut files);

    let mut offenders = Vec::new();
    for path in files {
        if path.ends_with("tmux_test_isolation_guard.rs") {
            continue; // this file mentions the markers as data
        }
        let Ok(src) = fs::read_to_string(&path) else {
            continue;
        };
        let destructive = src.contains("\"new-session\"") || src.contains("\"kill-session\"");
        let pins_socket = src.contains("\"-L\"") || src.contains("-S");
        if destructive && !pins_socket {
            offenders.push(path.display().to_string());
        }
    }

    assert!(
        offenders.is_empty(),
        "Integration test builds a destructive tmux command without an explicit \
         `-L`/`-S` private socket (see AGENTS.md \"Tmux Session Safety\"): {offenders:?}"
    );
}
