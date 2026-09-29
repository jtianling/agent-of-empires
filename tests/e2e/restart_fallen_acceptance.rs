use std::time::{Duration, Instant};

use serial_test::serial;

use crate::claude_model_support::require_sqlite3;
use crate::restart_fallen_support::{evidence, setup_fallen, tmux, wait_until, Fixture};

fn restart(f: &Fixture) {
    let old_pids = tmux(
        &f.h,
        &[
            "list-panes",
            "-t",
            &f.session,
            "-F",
            "#{pane_id}:#{pane_pid}",
        ],
    );
    f.h.send_keys("r");
    wait_until(&f.h, "both panes respawned", || {
        f.live()
            && tmux(
                &f.h,
                &[
                    "list-panes",
                    "-t",
                    &f.session,
                    "-F",
                    "#{pane_id}:#{pane_pid}",
                ],
            ) != old_pids
    });
    f.h.wait_for_absent("Restarting...", Duration::from_secs(20));
    f.h.wait_for_absent("dropped to shell", Duration::from_secs(20));
    f.healthy_for(4);
}

fn attach_and_return(f: &Fixture, round: usize) {
    evidence(
        &format!("lifecycle-{round}-restarted.txt"),
        &f.h.capture_screen(),
    );
    f.h.send_keys("Enter");
    wait_until(&f.h, "Enter attached", || {
        tmux(&f.h, &["list-clients", "-F", "#{session_name}"])
            .lines()
            .any(|line| line == f.session)
    });
    evidence(
        &format!("lifecycle-{round}-attached.txt"),
        &format!(
            "clients={}\n{}",
            tmux(
                &f.h,
                &["list-clients", "-F", "#{session_name}:#{client_tty}"]
            ),
            f.h.capture_screen()
        ),
    );
    tmux(&f.h, &["detach-client", "-s", &f.session]);
    f.h.wait_for("Agent of Empires");
    f.healthy_for(4);
    evidence(
        &format!("lifecycle-{round}-returned.txt"),
        &f.h.capture_screen(),
    );
}

fn stale_poll_race(f: &Fixture) {
    f.arm();
    f.exit_both();
    f.await_sample("race");
    f.h.send_keys("r");
    wait_until(&f.h, "restart completed while poll blocked", || f.live());
    f.h.wait_for_absent("Restarting...", Duration::from_secs(20));
    evidence("race-restarted-before-release.txt", &f.h.capture_screen());
    f.release();
    f.healthy_for(5);
    evidence("race-old-result-rejected.txt", &f.h.capture_screen());
}

fn true_exit(f: &Fixture) {
    f.h.send_keys_to_target(&f.panes[1], "C-c");
    f.h.wait_for_timeout("dropped to shell", Duration::from_secs(20));
    f.h.assert_screen_contains(&f.panes[1]);
    evidence("lifecycle-99-true-exit.txt", &f.h.capture_screen());
    assert_eq!(
        tmux(
            &f.h,
            &[
                "display-message",
                "-p",
                "-t",
                &f.panes[0],
                "#{pane_current_command}:#{pane_dead}"
            ]
        ),
        "codex:0"
    );
}

#[test]
#[serial]
fn dual_fallen_r_attach_repeat_and_true_exit() {
    crate::harness::require_tmux!();
    require_sqlite3!();
    let f = setup_fallen("restart_fallen_lifecycle");
    for round in 1..=3 {
        restart(&f);
        attach_and_return(&f, round);
    }
    stale_poll_race(&f);
    true_exit(&f);
    f.finish("lifecycle");
}

fn delayed_stub(f: &Fixture) {
    let runtime = f.native.parent().unwrap().join("native");
    std::fs::create_dir(&runtime).unwrap();
    let delayed_native = runtime.join("codex");
    std::fs::rename(&f.native, &delayed_native).unwrap();
    f.h.install_stub_script(
        "codex",
        &format!("#!/bin/sh\nsleep 2\nexec '{}'\n", delayed_native.display()),
    );
}

fn grace_race(f: &Fixture) {
    let restarted_at = Instant::now();
    f.h.send_keys("r");
    f.h.wait_for_timeout("Starting", Duration::from_secs(20));
    f.arm();
    f.await_sample("grace");
    let sampled_after = restarted_at.elapsed();
    assert!(
        sampled_after < Duration::from_secs(3),
        "fixture missed launch grace: {sampled_after:?}"
    );
    evidence(
        "grace-sampled.txt",
        &format!(
            "sample_after_r={sampled_after:?}\n{}\n{}",
            std::fs::read_to_string(f.gate.join("sample.txt")).unwrap(),
            f.h.capture_screen()
        ),
    );
    release_grace_sample(f, restarted_at);
}

fn release_grace_sample(f: &Fixture, restarted_at: Instant) {
    wait_until(&f.h, "native agents live before release", || f.live());
    while restarted_at.elapsed() < Duration::from_secs(4) {
        std::thread::sleep(Duration::from_millis(50));
    }
    evidence(
        "grace-live-before-release.txt",
        &format!(
            "release_after_r={:?}\n{}\n{}",
            restarted_at.elapsed(),
            tmux(
                &f.h,
                &[
                    "list-panes",
                    "-t",
                    &f.session,
                    "-F",
                    "#{pane_id}:#{pane_current_command}:#{pane_dead}"
                ]
            ),
            f.h.capture_screen()
        ),
    );
    f.release();
    std::thread::sleep(Duration::from_secs(2));
    evidence(
        "grace-after-release.txt",
        &format!(
            "{}\n{}",
            tmux(
                &f.h,
                &[
                    "list-panes",
                    "-t",
                    &f.session,
                    "-F",
                    "#{pane_id}:#{pane_current_command}:#{pane_dead}"
                ]
            ),
            f.h.capture_screen()
        ),
    );
    f.healthy_for(3);
}

#[test]
#[serial]
fn same_launch_grace_sample_must_not_latch_after_expiry() {
    crate::harness::require_tmux!();
    require_sqlite3!();
    let f = setup_fallen("restart_fallen_grace");
    delayed_stub(&f);
    grace_race(&f);
    f.finish("grace");
}
