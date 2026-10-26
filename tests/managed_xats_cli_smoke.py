"""Exercise the managed xats contract against a local fake daemon."""

import http.server
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import uuid


def main():
    binary = Path(sys.argv[1]).resolve()
    claims = {}
    released = set()
    mode = {"offline": False}
    calls = []
    token = "private-test-mcp-token"

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            calls.append((self.path, request))
            status, response = handle(self.path, request)
            data = json.dumps(response).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

    def handle(path, request):
        assert request["protocol_version"] == 1
        if path.endswith("/prepare"):
            if request["name"] == "taken":
                return 409, {"ok": False, "error": "identity_conflict",
                             "detail": "identity belongs to another instance"}
            key = request["identity_key"]
            if key in released:
                return 409, {"ok": False, "error": "stale_fence",
                             "detail": "released instance cannot be prepared again"}
            if request.get("previous"):
                assert request["previous"] == claims[key]
                claims[key] = {**claims[key], "instance_id": request["instance_id"],
                               "generation": claims[key]["generation"] + 1}
            if key not in claims:
                claims[key] = {"claim_id": str(uuid.uuid4()),
                               "instance_id": request["instance_id"], "generation": 1}
            fence = claims[key]
            result = envelope(fence, "prepared")
            result["mcp_token"] = token
            return 200, result
        fence = claims[request["identity_key"]]
        assert request["fence"] == fence
        if path.endswith("/status"):
            if mode["offline"]:
                return 502, {"ok": False, "error": "runtime_probe_failed",
                             "detail": "opencode_unreachable: local runtime stopped"}
            return 200, envelope(fence, "prepared")
        if path.endswith("/release"):
            released.add(request["identity_key"])
            return 200, envelope(fence, "released")
        raise AssertionError(path)

    def envelope(fence, state):
        return {"ok": True, "protocol_version": 1, "fence": fence,
                "state": state, "agent_id": None, "runtime": None}

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        with tempfile.TemporaryDirectory(prefix="aoe-managed-xats-") as directory:
            root = Path(directory)
            daemon = root / "xats"
            daemon.mkdir()
            (daemon / "daemon.pid").write_text(json.dumps({
                "pid": os.getpid(), "port": server.server_address[1],
            }))
            bin_dir = root / "bin"
            bin_dir.mkdir()
            sentinel = bin_dir / "tmux"
            sentinel.write_text('#!/bin/sh\necho "$*" >> "$HOME/tmux.log"\nexit 1\n')
            sentinel.chmod(0o700)
            context = root / "manager.json"
            env = {"HOME": str(root), "XDG_CONFIG_HOME": str(root / "config"),
                   "PATH": str(bin_dir) + os.pathsep + os.defpath,
                   "TMPDIR": str(root), "CROSS_AGENT_TEAMS_MCP_HOME": str(daemon)}

            def call(*args, success=True):
                result = subprocess.run(
                    [str(binary), "managed", "--context", str(context), *args],
                    env=env, capture_output=True, text=True, timeout=20,
                )
                assert token not in result.stdout + result.stderr
                data = json.loads(result.stdout)
                assert data["ok"] == success, (args, data, result.stderr)
                assert (result.returncode == 0) == success
                return data

            call("init")
            item = call("create", str(root), "--tool", "codex", "--title", "worker",
                        "--xats-name", "worker", "--xats-team", "test")["data"]
            assert item["xats"]["state"] == "prepared"
            assert item["xats"]["runtime_ready"] is False
            session_id = item["id"]
            assert call("show", session_id)["data"]["xats"]["state"] == "prepared"
            conflict = call("create", str(root), "--tool", "codex", "--title", "conflict",
                            "--xats-name", "taken", "--xats-team", "test",
                            success=False)["error"]
            assert conflict["session_id"]
            assert "identity_conflict" in conflict["message"]
            assert "identity belongs to another instance" in conflict["message"]
            assert not (root / "tmux.log").exists()
            before = len(calls)
            call("create", str(root), "--tool", "kimi", "--title", "unsupported",
                 "--xats-name", "kimi", "--xats-team", "test", success=False)
            assert len(calls) == before
            mode["offline"] = True
            stopped = call("stop", session_id)["data"]
            assert stopped["xats"]["state"] == "unavailable"
            assert stopped["xats"]["runtime_ready"] is False
            assert "opencode_unreachable" in stopped["xats"]["error"]
            observed = call("show", session_id)["data"]
            assert observed["status"] == "stopped"
            assert "runtime_probe_failed" in observed["xats"]["error"]
            mode["offline"] = False
            sessions_path = next(root.rglob("sessions.json"))
            records = json.loads(sessions_path.read_text())
            record = next(item for item in records if item["id"] == session_id)
            pending = str(uuid.uuid4())
            record["managed_xats"]["pending_restart"] = pending
            record["managed_xats"]["client_config"] = {
                "thread_id": str(uuid.uuid4()), "ws_url": "ws://127.0.0.1:1",
                "profile_name": "wrong", "profile_path": str(root / "wrong.config.toml"),
            }
            sessions_path.write_text(json.dumps(records))
            failure = call("remove", session_id, success=False)
            assert "profile path" in failure["error"]["message"]
            records = json.loads(sessions_path.read_text())
            record = next(item for item in records if item["id"] == session_id)
            assert record["managed_xats"]["claim"]["fence"]["instance_id"] == pending
            assert record["managed_xats"]["pending_restart"] is None
            record["managed_xats"]["client_config"] = None
            sessions_path.write_text(json.dumps(records))
            call("remove", session_id)
            assert calls[-1][0] == "/api/managed/release"
            assert "kill-session" not in (root / "tmux.log").read_text()
            print(json.dumps({"status": "PASS", "checks": [
                "prepare_fence", "not_ready", "status", "specific_conflict",
                "recoverable_session_id", "token_redaction", "unsupported_before_effects",
                "exact_release", "durable_release_retry", "offline_status_after_stop",
                "no_real_tmux",
            ]}))
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=5)


if __name__ == "__main__":
    main()
