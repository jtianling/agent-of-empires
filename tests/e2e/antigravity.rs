//! End-to-end tests for Antigravity launch, `r` (resume), and `c` (fresh clear
//! context while preserving the original xats identity).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;
use serial_test::serial;

use crate::antigravity_mock::{
    assert_pid_exited, install_native_agy, sqlite_query, wait_for_jsonl_count, MockXatsDaemon,
};
use crate::harness::{require_tmux, TuiTestHarness};

const AGENT_ID: &str = "11111111-2222-4333-8444-555555555555";
const CONV_1: &str = "2f52b5ab-0c1f-4121-9150-4595b571f562";
const CONV_2: &str = "a194f116-b0d6-48b6-9071-6d9ad9b7221d";

fn db_path(h: &TuiTestHarness) -> PathBuf {
    if cfg!(target_os = "linux") {
        h.home_path()
            .join(".config/agent-of-empires/profiles/default/aoe.db")
    } else {
        h.home_path()
            .join(".agent-of-empires/profiles/default/aoe.db")
    }
}

fn wait_for_slot_conversation(db: &Path, expected: &str) {
    let start = Instant::now();
    loop {
        if db.exists()
            && sqlite_query(db, "SELECT native_session_id FROM agent_slot WHERE slot=0;")
                == expected
        {
            return;
        }
        if start.elapsed() > Duration::from_secs(15) {
            let actual = if db.exists() {
                sqlite_query(db, "SELECT native_session_id FROM agent_slot WHERE slot=0;")
            } else {
                "<missing db>".to_string()
            };
            panic!("Timed out waiting for slot 0 native_session_id={expected}, actual={actual}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn install_xats_agy_stub(h: &TuiTestHarness, launch_log: &Path, bind_log: &Path) {
    h.install_stub_script(
        "xats-agy-mcp",
        "#!/bin/sh\n[ \"$1\" = --capabilities-json ] && printf '{\"defer_registration\":1,\"agentapi_channel\":1}\\n' && exit 0\nexit 0\n",
    );
    let bind_stub = format!(
        "#!/usr/bin/env python3\nimport json, os, sys\nwith open({bind_log:?}, 'a', encoding='utf-8') as f:\n    f.write(json.dumps({{'args': sys.argv[1:], 'conv': os.environ.get('ANTIGRAVITY_CONVERSATION_ID'), 'launch_id': os.environ.get('XATS_AGY_LAUNCH_ID')}}) + '\\n')\n",
        bind_log = bind_log.to_str().unwrap()
    );
    h.install_stub_script("xats-agy-channel-bind", &bind_stub);
    let xats_agy = format!(
        r#"#!/usr/bin/env python3
import json, os, signal, subprocess, sys, uuid

args = sys.argv[1:]
if args == ["--capabilities-json"]:
    print(json.dumps({{"defer_registration": 1, "agentapi_channel": 1}}))
    sys.exit(0)
if args == ["--help"]:
    print("usage: xats-agy")
    sys.exit(0)

split = args.index("--") if "--" in args else len(args)
head, tail = args[:split], args[split + 1:]
launch_id = str(uuid.uuid4())
env = dict(os.environ)
key = env.pop("XATS_AGY_IDENTITY_KEY", None)
env["XATS_AGY_LAUNCH_ID"] = launch_id
if "--agentapi" in head:
    env["XATS_AGY_AGENTAPI"] = "1"
    env["XATS_AGY_BIND_TOKEN"] = "deadbeef"
if "--defer-registration" in head:
    mode = "deferred"
    env["XATS_AGY_DEFER_REGISTRATION"] = "1"
else:
    mode = "named"
    env["XATS_AGY_NAME"] = head[head.index("--name") + 1]
    env["XATS_AGY_TEAM"] = head[head.index("--team") + 1]

proc = subprocess.Popen(["agy", *tail], env=env)
with open({launch_log:?}, "a", encoding="utf-8") as f:
    f.write(json.dumps({{
        "mode": mode, "head": head, "tail": tail, "launch_id": launch_id,
        "pid": os.getpid(), "has_key": bool(key), "key": key,
        "expected_agent": os.environ.get("AOE_ANTIGRAVITY_AGENT_ID"),
    }}) + "\n")

def stop(_sig, _frame):
    proc.terminate()
    try:
        proc.wait(timeout=2)
    except subprocess.TimeoutExpired:
        proc.kill()
    sys.exit(0)

# Ignore SIGHUP to reproduce an incumbent launcher that survives `respawn-pane -k`
# unless AoE explicitly reaps the verified previous process tree.
signal.signal(signal.SIGHUP, signal.SIG_IGN)
signal.signal(signal.SIGTERM, stop)
sys.exit(proc.wait())
"#,
        launch_log = launch_log.to_str().unwrap()
    );
    h.install_stub_script("xats-agy", &xats_agy);
}

fn install_agy_driver(h: &TuiTestHarness, mcp_log: &Path, next_conv: &Path) -> bool {
    let driver = h.home_path().join("agy_driver.py");
    let driver_body = format!(
        r#"import json, os, shlex, signal, subprocess, sys, time

os.environ["XATS_AGY_PID"] = str(os.getppid())
home = os.environ["HOME"]
argv = sys.argv[1:]
if "--conversation" in argv:
    conv_id = argv[argv.index("--conversation") + 1]
else:
    conv_id = open({next_conv:?}, encoding="utf-8").read().strip()

# Enforce single-instance conversation lock to catch "Conversation already open" races.
lock_path = os.path.join(home, f"conv_{{conv_id}}.lock")
try:
    fd = os.open(lock_path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
    os.write(fd, str(os.getpid()).encode())
    os.close(fd)
except FileExistsError:
    sys.stderr.write(f"Conversation already open: {{conv_id}}\n")
    sys.exit(42)

hooks = json.load(open(os.path.join(home, ".gemini/config/hooks.json"), encoding="utf-8"))
hook_cmd = hooks["agent-of-empires-pane-capture"]["SessionStart"][0]["command"]
subprocess.run(
    shlex.split(hook_cmd),
    input=json.dumps({{"conversationId": conv_id}}).encode(),
    check=True,
)

mcp_proc = None
if os.environ.get("AOE_ANTIGRAVITY_NAMED") == "1" and os.environ.get("AOE_ANTIGRAVITY_DEFERRED") == "0":
    mcp_cfg = json.load(open(os.path.join(home, ".gemini/config/mcp_config.json"), encoding="utf-8"))
    entry = mcp_cfg["mcpServers"]["aoe-xats"]
    mcp_proc = subprocess.Popen(
        [entry["command"], *entry.get("args", [])],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    def rpc(msg):
        mcp_proc.stdin.write(json.dumps(msg) + "\n")
        mcp_proc.stdin.flush()
        return None if "id" not in msg else json.loads(mcp_proc.stdout.readline())

    init = rpc({{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {{
        "protocolVersion": "2025-06-18", "capabilities": {{}}, "clientInfo": {{"name": "agy", "version": "1"}}
    }}}})
    rpc({{"jsonrpc": "2.0", "method": "notifications/initialized"}})
    tools = rpc({{"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {{}}}})
    inbox = rpc({{"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {{"name": "get_inbox", "arguments": {{}}}}}})
    with open({mcp_log:?}, "a", encoding="utf-8") as f:
        f.write(json.dumps({{
            "launch_id": os.environ.get("XATS_AGY_LAUNCH_ID"),
            "init_ok": "result" in init,
            "tools": [t["name"] for t in tools["result"]["tools"]],
            "inbox_ok": "result" in inbox and not inbox["result"].get("isError", False),
        }}) + "\n")

def stop(_sig, _frame):
    # Delay teardown slightly to verify AoE waits for graceful exit before reopening.
    time.sleep(0.2)
    if mcp_proc is not None:
        mcp_proc.terminate()
    try:
        os.unlink(lock_path)
    except FileNotFoundError:
        pass
    sys.exit(0)

signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGHUP, stop)
while True:
    time.sleep(1)
"#,
        next_conv = next_conv.to_str().unwrap(),
        mcp_log = mcp_log.to_str().unwrap(),
    );
    std::fs::write(&driver, driver_body).expect("write agy driver");
    let stub_dir = h.home_path().join("agy-bin");
    std::fs::create_dir_all(&stub_dir).expect("create agy-bin");
    let Some(native) = install_native_agy(&stub_dir, &driver) else {
        return false;
    };
    std::fs::copy(&native, h.stub_dir_path().join("agy")).expect("install native agy stub");
    true
}

fn assert_cli_ok(output: &std::process::Output, label: &str) {
    assert!(
        output.status.success(),
        "{label}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn launch_deferred_session(
    h: &TuiTestHarness,
    title: &str,
    launch_log: &Path,
    next_conv: &Path,
) -> (String, PathBuf) {
    std::fs::write(next_conv, CONV_1).expect("seed conv 1");
    let project = h.project_path();
    assert_cli_ok(
        &h.run_cli(&[
            "add",
            project.to_str().unwrap(),
            "-t",
            title,
            "-c",
            "agy",
            "--cross-agent-team",
        ]),
        "aoe add",
    );
    assert_cli_ok(
        &h.run_cli_in_tmux(&["session", "start", title]),
        "aoe session start",
    );
    let launches = wait_for_jsonl_count(launch_log, 1, "launch");
    assert_eq!(launches[0]["mode"], "deferred");
    let slot_key = launches[0]["key"].as_str().unwrap().to_string();
    let settings: Value = serde_json::from_str(
        &std::fs::read_to_string(h.home_path().join(".gemini/antigravity-cli/settings.json"))
            .expect("read antigravity settings"),
    )
    .expect("parse antigravity settings");
    let expected_workspace = project
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        str_vec(&settings["trustedWorkspaces"]).contains(&expected_workspace.as_str()),
        "expected {expected_workspace} in trustedWorkspaces: {settings}"
    );
    let db = db_path(h);
    wait_for_slot_conversation(&db, CONV_1);
    assert_eq!(
        sqlite_query(
            &db,
            "SELECT xats_identity_key FROM agent_slot WHERE slot=0;"
        ),
        slot_key
    );
    (slot_key, db)
}

fn assert_named_mcp_run(run: &Value) {
    assert_eq!(run["init_ok"], true);
    assert_eq!(run["inbox_ok"], true);
    let names: Vec<&str> = run["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(names.contains(&"broadcast_to_role"));
    assert!(names.contains(&"get_delivery_status"));
    assert!(names.contains(&"cancel_unread_alert"));
    assert!(!names.contains(&"register_agent"));
}

fn str_vec(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect()
}

fn assert_agentapi_launch(launch: &Value, expect_prompt: bool, expected_conv: Option<&str>) {
    let head = str_vec(&launch["head"]);
    let tail = str_vec(&launch["tail"]);
    assert!(head.contains(&"--agentapi"), "head={head:?}");
    let idx = tail
        .iter()
        .position(|a| *a == "--settings")
        .expect("missing --settings");
    let settings: Value = serde_json::from_str(tail[idx + 1]).expect("valid settings json");
    assert!(str_vec(&settings["permissions"]["allow"])
        .contains(&"command(regex:^xats-agy-channel-bind$)"));
    assert_eq!(tail.contains(&"-i"), expect_prompt, "tail={tail:?}");
    match expected_conv {
        Some(conv) => {
            let cidx = tail
                .iter()
                .position(|a| *a == "--conversation")
                .expect("missing --conversation");
            assert_eq!(tail[cidx + 1], conv);
        }
        None => assert!(!tail.contains(&"--conversation"), "tail={tail:?}"),
    }
}

#[test]
#[serial]
fn antigravity_launch_resume_and_fresh_preserve_xats_identity() {
    require_tmux!();
    let mut h = TuiTestHarness::new("antigravity_lifecycle");
    let launch_log = h.home_path().join("launches.jsonl");
    let bind_log = h.home_path().join("bind_updates.jsonl");
    let mcp_log = h.home_path().join("mcp.jsonl");
    let next_conv = h.home_path().join("next_conv.txt");
    install_xats_agy_stub(&h, &launch_log, &bind_log);
    if !install_agy_driver(&h, &mcp_log, &next_conv) {
        eprintln!("Skipping test: C compiler unavailable for native agy stub");
        return;
    }

    let daemon = MockXatsDaemon::start(AGENT_ID, "aoe-agy", "aoe");
    daemon.write_pid_file(h.home_path());
    let (slot_key, db) = launch_deferred_session(&h, "agy-e2e", &launch_log, &next_conv);
    let initial = wait_for_jsonl_count(&launch_log, 1, "initial deferred");
    assert_agentapi_launch(&initial[0], false, None);
    let updates = wait_for_jsonl_count(&bind_log, 1, "initial conversation update");
    assert_eq!(
        updates[0]["args"],
        serde_json::json!(["--update-conversation"])
    );
    assert_eq!(updates[0]["conv"], CONV_1);

    // 1. Live `r` (Resume): keeps CONV_1 and switches to named xats-agy + MCP bridge.
    daemon.bind_identity(&slot_key);
    h.spawn_tui();
    h.wait_for("agy-e2e");
    h.send_keys("r");

    let launches = wait_for_jsonl_count(&launch_log, 2, "resume launch");
    assert_pid_exited(launches[0]["pid"].as_u64().unwrap());
    assert_eq!(launches[1]["mode"], "named");
    assert_eq!(launches[1]["expected_agent"], AGENT_ID);
    assert_agentapi_launch(&launches[1], true, Some(CONV_1));
    let mcp_runs = wait_for_jsonl_count(&mcp_log, 1, "named MCP run");
    assert_named_mcp_run(&mcp_runs[0]);

    // 2. Live `c` (Fresh / clear context): drops `--conversation`, restores same xats identity.
    std::fs::write(&next_conv, CONV_2).expect("seed conv 2");
    h.send_keys("c");

    let launches = wait_for_jsonl_count(&launch_log, 3, "fresh launch");
    assert_pid_exited(launches[1]["pid"].as_u64().unwrap());
    assert_eq!(launches[2]["mode"], "named");
    assert_eq!(launches[2]["expected_agent"], AGENT_ID);
    assert_agentapi_launch(&launches[2], true, None);
    wait_for_slot_conversation(&db, CONV_2);
    assert_eq!(
        sqlite_query(
            &db,
            "SELECT xats_identity_key FROM agent_slot WHERE slot=0;"
        ),
        slot_key
    );
    let mcp_runs = wait_for_jsonl_count(&mcp_log, 2, "fresh MCP run");
    assert_named_mcp_run(&mcp_runs[1]);
    assert_ne!(mcp_runs[0]["launch_id"], mcp_runs[1]["launch_id"]);

    // 3. Subsequent `r` resumes the new conversation CONV_2.
    h.send_keys("r");
    let launches = wait_for_jsonl_count(&launch_log, 4, "post-fresh resume");
    assert_agentapi_launch(&launches[3], true, Some(CONV_2));
    let mcp_runs = wait_for_jsonl_count(&mcp_log, 3, "post-fresh resume MCP");
    assert_named_mcp_run(&mcp_runs[2]);

    // 4. Cold recovery via `c` (after stopping the session) clears conversation and keeps identity.
    let stop = h.run_cli_in_tmux(&["session", "stop", "agy-e2e"]);
    assert!(
        stop.status.success(),
        "aoe session stop failed: {}",
        String::from_utf8_lossy(&stop.stderr)
    );
    assert_pid_exited(launches[3]["pid"].as_u64().unwrap());
    std::fs::write(&next_conv, CONV_1).expect("seed cold fresh conv");
    h.wait_for("[recoverable]");
    h.send_keys("c");

    let launches = wait_for_jsonl_count(&launch_log, 5, "cold fresh recovery");
    assert_eq!(launches[4]["mode"], "named");
    assert_eq!(launches[4]["expected_agent"], AGENT_ID);
    assert_agentapi_launch(&launches[4], true, None);
    wait_for_slot_conversation(&db, CONV_1);
    let updates = wait_for_jsonl_count(&bind_log, 5, "all conversation updates");
    assert_eq!(updates[4]["conv"], CONV_1);
    let mcp_runs = wait_for_jsonl_count(&mcp_log, 4, "cold fresh MCP");
    assert_named_mcp_run(&mcp_runs[3]);
    let regs = daemon.state.lock().unwrap().named_registrations.clone();
    assert!(
        regs.len() >= 4,
        "expected named registrations across r/c/r/cold-c: {regs:?}"
    );
    for reg in &regs {
        assert_eq!(reg["identity_key"], slot_key);
        assert_eq!(reg["agent_type"], "antigravity");
        assert_eq!(reg["name"], "aoe-agy");
        assert_eq!(reg["team"], "aoe");
    }

    // Stop cleanly so the SIGHUP-ignoring stub does not outlive the test harness.
    let stop = h.run_cli_in_tmux(&["session", "stop", "agy-e2e"]);
    assert!(
        stop.status.success(),
        "final aoe session stop failed: {}",
        String::from_utf8_lossy(&stop.stderr)
    );
    assert_pid_exited(launches[4]["pid"].as_u64().unwrap());
}
