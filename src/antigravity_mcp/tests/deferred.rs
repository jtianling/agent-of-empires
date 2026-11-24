use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::Stdio;

use super::*;

fn fixture(dir: &Path, script: &str) -> PathBuf {
    let path = dir.join(PROGRAM);
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[test]
fn capability_requires_exact_integer_and_a_successful_process() {
    let dir = tempfile::tempdir().unwrap();
    let probe = |body: &str| {
        let path = fixture(
            dir.path(),
            &format!("#!/bin/sh\n[ \"$#:$1\" = '1:--capabilities-json' ] || exit 2\n{body}\n"),
        );
        require_deferred_capability(&path)
    };
    probe("echo '{\"defer_registration\":1}'").unwrap();
    for body in [
        "echo '{\"defer_registration\":true}'",
        "echo '{\"defer_registration\":\"1\"}'",
        "echo '{\"defer_registration\":1.0}'",
        "echo '{\"defer_registration\":2}'",
        "echo '{}'",
        "echo '[1]'",
        "echo '{\"defer_registration\":0,\"defer_registration\":1}'",
        "echo '{\"defer_registration\":1}'; exit 3",
        "echo '{\"defer_registration\":1}'; echo 'extra'",
    ] {
        assert!(probe(body).is_err(), "{body}");
    }
    let error = probe("if read -r request; then exit 3; fi").unwrap_err();
    assert!(format!("{error:#}").contains("Invalid deferred capability response"));
    let error = probe("echo 'unsupported option' >&2; exit 4").unwrap_err();
    assert!(format!("{error:#}").contains("unsupported option"));
}

#[test]
fn unresponsive_and_oversized_probes_are_stopped() {
    let dir = tempfile::tempdir().unwrap();
    for (body, expected) in [
        ("exec /bin/sleep 30", "timed out"),
        ("while :; do echo 'oversized'; done", "size limit"),
    ] {
        let path = fixture(dir.path(), &format!("#!/bin/sh\n{body}\n"));
        let error = require_deferred_capability(&path).unwrap_err();
        assert!(format!("{error:#}").contains(expected));
    }
}

#[test]
fn deferred_markers_and_launch_environment_are_strict() {
    assert!(!requested(None).unwrap());
    assert!(!requested(Some(OsStr::new("0"))).unwrap());
    assert!(requested(Some(OsStr::new("1"))).unwrap());
    assert!(requested(Some(OsStr::new(""))).is_err());
    let launch = uuid::Uuid::new_v4().to_string();
    let vars = HashMap::from([
        ("AOE_ANTIGRAVITY_NAMED", "1"),
        ("XATS_AGY_DEFER_REGISTRATION", "1"),
        ("XATS_AGY_LAUNCH_ID", launch.as_str()),
        ("XATS_AGY_PID", "123"),
    ]);
    let check =
        |env: &HashMap<&str, &str>| validate_launch(|field| env.get(field).map(OsString::from));
    check(&vars).unwrap();
    for field in vars.keys() {
        let mut missing = vars.clone();
        missing.remove(field);
        assert!(check(&missing).is_err());
    }
    for (field, value) in [
        ("XATS_AGY_DEFER_REGISTRATION", "0"),
        ("XATS_AGY_LAUNCH_ID", "bad"),
        ("XATS_AGY_PID", "0123"),
        ("XATS_AGY_PID", "1"),
        ("XATS_AGY_NAME", ""),
        ("XATS_AGY_TEAM", "dev"),
        ("XATS_IDENTITY_KEY", "foreign"),
        (IDENTITY, "foreign"),
    ] {
        let mut changed = vars.clone();
        changed.insert(field, value);
        assert!(check(&changed).is_err(), "{field}");
    }
}

#[test]
fn exec_inherits_stdio_pid_and_exclusive_slot_lock() {
    const FIXTURE: &str = "AOE_TEST_AGY_DEFERRED_BRIDGE_DIR";
    const KEY: &str = "fixture-slot";
    if let Some(dir) = std::env::var_os(FIXTURE) {
        let dir = Path::new(&dir);
        let lock = lease::acquire(dir, KEY).unwrap();
        PreparedBridge::from_env().unwrap().exec(KEY, lock).unwrap();
        unreachable!();
    }
    let dir = tempfile::tempdir().unwrap();
    let source = fixture(
        dir.path(),
        r#"#!/usr/bin/env bridge-shell
if [ "$1" = "--capabilities-json" ]; then
    [ -z "${XATS_AGY_IDENTITY_KEY+x}${XATS_IDENTITY_KEY+x}${XATS_AGY_LAUNCH_ID+x}" ] || exit 2
    echo '{"defer_registration":1}'
    exit 0
fi
[ "$#" -eq 0 ] && [ "$XATS_AGY_IDENTITY_KEY" = fixture-slot ] || exit 1
printf '\nREADY %s\n' "$$"
read -r request && echo "REPLY $request"
"#,
    );
    let bridge = dir.path().join("dist/agy-mcp.js");
    std::fs::create_dir(bridge.parent().unwrap()).unwrap();
    std::fs::rename(source, &bridge).unwrap();
    std::os::unix::fs::symlink("/bin/sh", dir.path().join("bridge-shell")).unwrap();
    let agy_dir = tempfile::tempdir().unwrap();
    let decoy = fixture(agy_dir.path(), "#!/bin/sh\nexit 9\n");
    std::os::unix::fs::symlink(decoy, agy_dir.path().join("bridge-shell")).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "antigravity_mcp::deferred::tests::exec_inherits_stdio_pid_and_exclusive_slot_lock",
            "--nocapture",
        ])
        .env(FIXTURE, dir.path())
        .env(
            "PATH",
            std::env::join_paths([agy_dir.path(), dir.path()]).unwrap(),
        )
        .env("AOE_ANTIGRAVITY_MCP_BINARY", bridge.canonicalize().unwrap())
        .env("AOE_ANTIGRAVITY_MCP_PATH", dir.path())
        .env("XATS_AGY_LAUNCH_ID", "fixture-launch")
        .env("XATS_IDENTITY_KEY", "fixture-parent-key")
        .env(IDENTITY, "fixture-parent-key")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let ready = lines
        .by_ref()
        .map(Result::unwrap)
        .find(|s| s.starts_with("READY "));
    assert_eq!(ready, Some(format!("READY {}", child.id())));
    assert!(lease::acquire(dir.path(), KEY)
        .unwrap_err()
        .to_string()
        .contains("already active"));
    writeln!(child.stdin.take().unwrap(), "ping").unwrap();
    assert_eq!(lines.next().unwrap().unwrap(), "REPLY ping");
    assert!(child.wait().unwrap().success());
    lease::acquire(dir.path(), KEY).unwrap();
}

#[test]
fn pinned_bridge_rejects_relative_paths_symlinks_and_replacements() {
    let dir = tempfile::tempdir().unwrap();
    let original = fixture(dir.path(), "#!/bin/sh\necho '{\"defer_registration\":1}'\n");
    let path = original.canonicalize().unwrap();
    assert!(file_identity(Path::new(PROGRAM)).is_err());
    let alias = path.with_extension("link");
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    assert!(file_identity(&alias).is_err());
    let bridge = PreparedBridge::check(path.clone(), "/usr/bin:/bin".into()).unwrap();
    std::fs::rename(&path, path.with_extension("old")).unwrap();
    std::fs::write(&path, []).unwrap();
    let lock = lease::acquire(dir.path(), "fixture").unwrap();
    let error = bridge.exec("fixture", lock).unwrap_err();
    assert!(error.to_string().contains("executable changed"));
}
