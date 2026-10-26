"""Check pane authorization without connecting to a real tmux server."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import uuid


def main():
    binary = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory(prefix="aoe-pane-smoke-") as directory:
        root = Path(directory)
        (root / "bin").mkdir()
        sentinel = root / "bin/tmux"
        sentinel.write_text('#!/bin/sh\necho "$*" >> "$HOME/tmux.log"\nexit 1\n')
        sentinel.chmod(0o700)
        env = {"HOME": str(root), "XDG_CONFIG_HOME": str(root / "config"),
               "PATH": str(root / "bin") + os.pathsep + os.defpath,
               "TMPDIR": str(root)}
        first, second = root / "first.json", root / "second.json"
        log = root / "tmux.log"

        def call(*args, context=first, success=True):
            result = subprocess.run([str(binary), "-p", "default", "managed",
                                     "--context", str(context), *args],
                                    env=env, text=True, capture_output=True, timeout=20)
            data = json.loads(result.stdout)
            assert data["ok"] == success, (args, data, result.stderr)
            assert (result.returncode == 0) == success
            return data

        owner = call("init")["data"]["owner"]
        other = call("init", context=second)["data"]["owner"]
        session = call("create", str(root), "--tool", "shell", "--title", "parent")["data"]
        sid = session["id"]
        data_path = next(root.rglob("sessions.json"))
        records = json.loads(data_path.read_text())
        pane_id = uuid.uuid4().hex[:16]
        foreign = {"id": pane_id, "owner": other, "slot": None, "tmux_pane": None,
                   "config": records[0]["primary_pane"], "identity_key": str(uuid.uuid4()),
                   "xats": None, "native_session_id": None, "slot_generation": 0,
                   "removing": False}
        owned = {**foreign, "id": uuid.uuid4().hex[:16], "owner": owner,
                 "identity_key": str(uuid.uuid4())}
        human = {**foreign, "id": uuid.uuid4().hex[:16], "owner": None,
                 "identity_key": str(uuid.uuid4())}
        records[0]["managed_panes"] = [foreign, owned, human]
        data_path.write_text(json.dumps(records))
        before = data_path.read_bytes()
        for command in [
            ("pane", "send", sid, pane_id, "message"),
            ("pane", "capture", sid, pane_id),
            ("pane", "remove", sid, pane_id),
            ("stop", sid), ("restart", sid), ("remove", sid),
        ]:
            call(*command, success=False)
            assert data_path.read_bytes() == before
            assert not log.exists(), "Foreign pane rejection reached tmux"
        for pane in [pane_id[:8], "%0", "left", "", "f" * 32]:
            call("pane", "send", sid, pane, "message", success=False)
            assert data_path.read_bytes() == before
            assert not log.exists(), "Invalid pane ID reached tmux"
        for command in [
            ("pane", "create", sid, "--tool", "shell"),
            ("pane", "send", sid, sid, "message"),
            ("pane", "capture", sid, sid),
            ("pane", "remove", sid, sid),
        ]:
            call(*command, context=second, success=False)
            assert data_path.read_bytes() == before
            assert not log.exists(), "Foreign session rejection reached tmux"
        call("pane", "create", sid, "--tool", "kimi", "--xats-name", "worker",
             "--xats-team", "test", success=False)
        assert data_path.read_bytes() == before and not log.exists()
        transfer = subprocess.run([str(binary), "-p", "default", "session", "set-owner",
                                   sid, "--context", str(second)], env=env,
                                  text=True, capture_output=True, timeout=20)
        assert transfer.returncode == 0, transfer.stderr
        updated = json.loads(data_path.read_text())[0]
        assert updated["managed_owner"] == other
        assert updated["managed_panes"][0]["owner"] == other
        assert updated["managed_panes"][1]["owner"] == other
        assert updated["managed_panes"][2]["owner"] is None
        call("pane", "send", sid, pane_id, "message", success=False)
        assert not log.exists()
        assert owner != other
        print(json.dumps({"status": "PASS", "checks": ["pane_owner_rejection",
            "parent_bypass_rejection", "exact_pane_id", "session_owner_rejection",
            "unsupported_before_effects", "ownership_transfer", "no_real_tmux"]}))


if __name__ == "__main__":
    main()
