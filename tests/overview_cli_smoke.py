"""Exercise read-only inventory with fake tmux and isolated application state."""

import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile


def main():
    binary = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory(prefix="aoe-overview-smoke-") as directory:
        root = Path(directory)
        bin_dir = root / "bin"
        bin_dir.mkdir()
        tmux = bin_dir / "tmux"
        tmux.write_text(
            '#!/bin/sh\n'
            '[ -z "$TMUX" ] && [ -z "$TMUX_PANE" ] || exit 99\n'
            '[ "$1" = "-L" ] && [ "$3" = "list-panes" ] || exit 98\n'
            'printf "%s\\n" "$*" >> "$HOME/tmux.log"\n'
            'echo "no server running on test socket" >&2\nexit 1\n'
        )
        tmux.chmod(0o700)
        env = {"HOME": str(root), "XDG_CONFIG_HOME": str(root / "config"),
               "PATH": str(bin_dir) + os.pathsep + os.defpath,
               "TMPDIR": str(root), "TMUX": "must-be-removed", "TMUX_PANE": "%99",
               "AGENT_OF_EMPIRES_DEBUG": "1"}
        app = root / ("config/agent-of-empires" if sys.platform == "linux"
                      else ".agent-of-empires")

        def call(*args, success=True):
            result = subprocess.run([str(binary), *args], env=env,
                                    text=True, capture_output=True, timeout=20)
            output = json.loads(result.stdout)
            assert (result.returncode == 0) == success, (args, result.stderr)
            assert output["ok"] == success, (args, output)
            return output.get("data", output)

        assert call("overview", "--all-profiles")["profiles"] == []
        assert not app.exists(), "Empty inventory created application state"
        assert call("profile", "instances")["instances"] == []
        assert not app.exists(), "Instance query created application state"
        call("overview", "--bad-argument", success=False)
        call("profile", "instances", "--bad-argument", success=False)
        call("-p", "../escape", "overview", success=False)
        profile = app / "profiles/alpha"
        profile.mkdir(parents=True)
        records = [{"id": "0123456789abcdef", "title": "human", "project_path": "/project",
                    "tool": "shell", "status": "running", "created_at": "2026-10-27T00:00:00Z"}]
        (profile / "sessions.json").write_text(json.dumps(records))
        (profile / "groups.json").write_text(json.dumps([{"path": "empty", "name": "empty"}]))
        (app / "profiles/empty").mkdir()
        broken = app / "profiles/broken"
        broken.mkdir()
        (broken / "sessions.json").write_text("not JSON")
        for name, contents in [("truncated", ""), ("whitespace", " \n\t")]:
            truncated = app / "profiles" / name
            truncated.mkdir()
            (truncated / "sessions.json").write_text(contents)
            (truncated / "groups.json").write_text(contents)
        instances = profile / ".instances"
        instances.mkdir()
        (instances / str(os.getpid())).write_text("")
        (instances / "99999999").write_text(json.dumps({"version": 1, "pid": 99999999,
            "profile": "alpha", "started_at": "deliberately-wrong"}))
        (instances / "invalid").write_text("invalid")
        connection = sqlite3.connect(profile / "aoe.db")
        connection.execute("PRAGMA journal_mode=WAL")
        connection.execute("CREATE TABLE agent_slot (instance_id TEXT, slot INTEGER, agent TEXT, cwd TEXT, tmux_pane TEXT)")
        connection.commit()
        connection.execute("INSERT INTO agent_slot VALUES (?, 1, 'shell', '/project', '%9')", (records[0]["id"],))
        connection.commit()

        def snapshot():
            return {str(path.relative_to(app)): hashlib.sha256(path.read_bytes()).hexdigest()
                    for path in app.rglob("*") if path.is_file()}

        before = snapshot()
        result = call("overview", "--all-profiles")
        assert result["partial"] is True
        for name in ["truncated", "whitespace"]:
            damaged = next(p for p in result["profiles"] if p["profile"] == name)
            assert damaged["partial"] is True
            assert damaged["sessions"] is None and damaged["groups"] is None
            assert {e["source"] for e in damaged["errors"]} == {"sessions", "groups"}
        alpha = next(p for p in result["profiles"] if p["profile"] == "alpha")
        assert alpha["groups"][0]["path"] == "empty"
        session = alpha["sessions"][0]
        assert session["status"] == "running" and session["status_source"] == "stored"
        assert session["live"] is False
        assert len(session["panes"]) == 2, "Active WAL row was not included"
        assert all(p["pane_id"] is None for p in session["panes"])
        assert snapshot() == before, "Overview changed application state"
        queried = call("profile", "instances")
        statuses = {p["registration"]: p["status"] for p in queried["instances"]}
        assert statuses[str(os.getpid())] == "unverified"
        assert statuses["99999999"] == "stale" and statuses["invalid"] == "invalid"
        assert snapshot() == before, "Instance query changed application state"
        assert call("-p", "missing", "overview")["profiles"][0]["sessions"] == []
        assert snapshot() == before, "Missing profile query created state"
        tmux.write_text(
            '#!/bin/sh\n'
            '[ -z "$TMUX" ] && [ -z "$TMUX_PANE" ] || exit 99\n'
            '[ "$1" = "-L" ] && [ "$3" = "list-panes" ] || exit 98\n'
            'echo "/tmp/private-smoke|123|1700000000|%9|aoe_human_01234567|%9||0|0|0"\n'
        )
        live_result = call("-p", "alpha", "overview")
        live_session = live_result["profiles"][0]["sessions"][0]
        assert live_session["live"] is True, live_result
        observed_pane = next(p for p in live_session["panes"] if p["live"])
        assert observed_pane["pane_id"] is None
        assert observed_pane["xats"]["identity"]["status"] == "unavailable"
        assert observed_pane["xats"]["runtime"]["status"] == "unknown"
        assert snapshot() == before
        connection.close()
        print(json.dumps({"status": "PASS", "checks": ["empty_no_write", "json_errors",
            "profile_validation", "stored_vs_live", "manual_id_null", "WAL_snapshot",
            "partial_failure", "legacy_and_stale_instances", "no_state_changes",
            "no_real_tmux"]}))


if __name__ == "__main__":
    main()
