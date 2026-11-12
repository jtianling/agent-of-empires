import http.server
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import threading
import uuid

TOKEN = "left-smoke-key-smoke-bearer-token"


TMUX_STUB = '''#!{python}
import json, os, pathlib, sys
root = pathlib.Path(os.environ["AOE_TEST_ROOT"])
args = sys.argv[1:]
if args[:1] == ["-L"]:
    args = args[2:]
with (root / "events.jsonl").open("a") as log:
    log.write(json.dumps({{"tmux": args}}) + "\\n")
command = args[0] if args else ""
if command == "list-panes":
    if "#{{socket_path}}" in args[-1]:
        inventory = (root / "inventory").read_text()
        if not inventory:
            print("can't find session: test", file=sys.stderr)
            sys.exit(1)
        print(inventory, end="")
    elif args[-1] == "#{{pane_id}}":
        print("%1\\n%2")
elif command == "has-session":
    sys.exit(int((root / "killed").exists()
                 or not (root / "inventory").read_text()))
elif command == "kill-session":
    (root / "killed").touch()
elif command == "-V":
    print("tmux 3.4")
'''


def create_environment(binary, root, server, mode):
    root.mkdir()
    (root / "bin").mkdir()
    stub = root / "bin/tmux"
    stub.write_text(TMUX_STUB.format(python=sys.executable))
    stub.chmod(0o700)
    (root / "inventory").write_text("")
    (root / "xats").mkdir()
    if mode != "no-daemon":
        (root / "xats/daemon.pid").write_text(json.dumps({
            "pid": os.getpid(), "port": server.server_address[1],
        }))
    env = {
        "HOME": str(root), "XDG_CONFIG_HOME": str(root / "config"),
        "PATH": str(root / "bin") + os.pathsep + os.defpath,
        "TMPDIR": str(root), "AOE_TEST_ROOT": str(root),
        "CROSS_AGENT_TEAMS_MCP_HOME": str(root / "xats"),
        "CROSS_AGENT_TEAMS_MCP_TOKEN": TOKEN,
        "AGENT_OF_EMPIRES_DEBUG": "1",
    }

    def call(*args):
        return subprocess.run(
            [str(binary), "-p", "default", *args], env=env,
            capture_output=True, text=True, timeout=30,
        )

    return call


def create_session(root, call, keys, live, pane_count):
    added = call("add", str(root), "-t", "release-pair", "-c", "codex")
    assert added.returncode == 0, added.stderr
    sessions_path = next(root.rglob("sessions.json"))
    records = json.loads(sessions_path.read_text())
    record = next(item for item in records if item["title"] == "release-pair")
    session_id = record["id"]
    record["primary_pane"]["cross_agent_team"] = keys
    record["cross_agent_team"] = keys
    record["xats_identity_key"] = "left-smoke-key" if keys else None
    sessions_path.write_text(json.dumps(records))
    db_path = next(root.rglob("aoe.db"))
    with sqlite3.connect(db_path) as connection:
        for index, agent, native in [
            (0, "codex", str(uuid.uuid4())), (1, "opencode", "ses_right"),
        ]:
            connection.execute(
                "INSERT OR REPLACE INTO agent_slot "
                "(instance_id,slot,agent,native_session_id,cwd,tmux_pane,"
                "xats_identity_key,cross_agent_team,pane_config_version,last_seen_at) "
                "VALUES(?,?,?,?,?,?,?,?,1,1)",
                (session_id, index, agent, native, str(root), f"%{index + 1}",
                 ["left-smoke-key", "right-smoke-key"][index] if keys else "", int(keys)),
            )
    session = f"aoe_release-pair_{session_id[:8]}"
    inventory = "".join(
        f"{root}/private-socket|123|1793040000|%{index + 1}|{session}|%1||"
        f"{index}|{index * 80}|0\n" for index in range(pane_count)
    )
    (root / "inventory").write_text(inventory if live else "")
    return session_id


def exercise(binary, root, server, state, mode, keys=True, live=True, pane_count=2):
    call = create_environment(binary, root, server, mode)
    session_id = create_session(root, call, keys, live, pane_count)
    (root / "events.jsonl").write_text("")
    state.update(root=root, mode=mode)
    result = call("remove", session_id)
    events = [json.loads(line) for line in (root / "events.jsonl").read_text().splitlines()]
    assert_deletion(root, session_id, mode, live, result, events)
    assert_release_request(root, keys, events)
    assert_diagnostics(root, mode, result)


def assert_deletion(root, session_id, mode, live, result, events):
    successful = mode in {"success", "no-daemon", "unregistered"}
    assert (result.returncode == 0) == successful, (mode, result.stdout, result.stderr)
    remaining = json.loads(next(root.rglob("sessions.json")).read_text())
    assert any(item["id"] == session_id for item in remaining) != successful
    killed = [index for index, event in enumerate(events)
              if event.get("tmux", [])[:1] == ["kill-session"]]
    releases = [(index, event) for index, event in enumerate(events)
                if event.get("http") == "/api/runtime/panes/release"]
    if successful and live:
        assert killed, (mode, events)
        if mode != "no-daemon":
            assert len(releases) == 1 and releases[0][0] < killed[0]
    if not successful:
        assert not killed
        with sqlite3.connect(next(root.rglob("aoe.db"))) as connection:
            assert connection.execute(
                "SELECT count(*) FROM agent_slot WHERE instance_id=?", (session_id,),
            ).fetchone()[0] == 2


def assert_release_request(root, keys, events):
    releases = [event for event in events
                if event.get("http") == "/api/runtime/panes/release"]
    if not releases:
        return
    targets = releases[0]["request"]["targets"]
    assert [target["tmux"]["pane_id"] for target in targets] == ["%1", "%2"]
    assert all(target["tmux"]["socket_path"] == f"{root}/private-socket"
               for target in targets)
    assert targets[1]["opencode_session_id"] == "ses_right"
    assert targets[0].get("identity_key") == ("left-smoke-key" if keys else None)
    assert targets[1].get("identity_key") == ("right-smoke-key" if keys else None)


def assert_diagnostics(root, mode, result):
    if mode == "conflict":
        assert "identity_mismatch" in result.stderr and "moved" in result.stderr
    if mode == "unsupported":
        assert "unsupported (HTTP 404)" in result.stderr
    if mode == "held":
        assert "pane is missing" in result.stderr
    diagnostics = result.stdout + result.stderr
    diagnostics += "".join(path.read_text() for path in root.rglob("debug.log"))
    for secret in ["left-smoke-key", "right-smoke-key", "smoke-bearer-token"]:
        assert secret not in diagnostics, (mode, secret)


def make_handler(state):
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_args):
            return

        def do_POST(self):
            request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            assert self.headers["Authorization"] == f"Bearer {TOKEN}"
            with (state["root"] / "events.jsonl").open("a") as log:
                log.write(json.dumps({"http": self.path, "request": request}) + "\n")
            status, response = reply(state, self.path, request)
            body = json.dumps(response).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    return Handler


def reply(state, path, request):
    mode = state["mode"]
    if path == "/api/identity-key/lookup":
        if mode == "unregistered":
            return 200, {"ok": False, "error": "not_found"}
        return 200, {"ok": True, "protocol_version": 1,
                     "identity_key": request["identity_key"], "holder": {
                         "team": "test", "name": "held", "agent_type": "codex",
                         "active": False,
                     }}
    assert path == "/api/runtime/panes/release", path
    if mode == "unsupported":
        return 404, {"message": "Route not found", "statusCode": 404, "error": "Not Found"}
    if mode == "conflict":
        return 409, {"ok": False, "protocol_version": 1, "error": "identity_mismatch",
                     "detail": f"moved: left-smoke-key right-smoke-key {TOKEN}",
                     "target_index": 1, "retryable": False}
    results = [{"index": index, "state": "released", "agent": {
        "agent_id": str(uuid.uuid4()), "name": f"worker-{index}",
        "team": "test", "agent_type": "codex",
    }} for index, _ in enumerate(request["targets"])]
    if mode == "partial":
        results.pop()
    return 200, {"ok": True, "protocol_version": 1, "results": results}


def main():
    binary = Path(sys.argv[1]).resolve()
    state = {}

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), make_handler(state))
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        with tempfile.TemporaryDirectory(prefix="aoe-release-smoke-") as directory:
            root = Path(directory)
            for mode in ["success", "conflict", "unsupported", "partial"]:
                exercise(binary, root / mode, server, state, mode)
            exercise(binary, root / "manual", server, state, "success", keys=False)
            exercise(binary, root / "no-daemon", server, state, "no-daemon", keys=False)
            exercise(binary, root / "many-plain-panes", server, state, "no-daemon",
                     keys=False, pane_count=17)
            exercise(binary, root / "held", server, state, "held", live=False)
            exercise(binary, root / "unregistered", server, state, "unregistered", live=False)
        print(json.dumps({"status": "PASS", "scenarios": 9, "real_tmux_calls": 0}))
    finally:
        server.shutdown()
        server.server_close()
        worker.join()


if __name__ == "__main__":
    main()
