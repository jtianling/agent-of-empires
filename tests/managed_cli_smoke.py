"""Exercise the managed CLI with a tmux sentinel and an isolated home."""

import concurrent.futures
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    binary = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory(prefix="aoe-managed-smoke-") as directory:
        root = Path(directory)
        bin_dir = root / "bin"
        bin_dir.mkdir()
        log = root / "tmux.log"
        tmux = bin_dir / "tmux"
        tmux.write_text(
            '#!/bin/sh\n'
            'printf "%s\\n" "$*" >> "$HOME/tmux.log"\n'
            'printf "%s\\n" "no server running on test socket" >&2\n'
            'exit 1\n'
        )
        tmux.chmod(0o700)
        env = {
            "HOME": str(root),
            "XDG_CONFIG_HOME": str(root / "config"),
            "PATH": str(bin_dir) + os.pathsep + os.defpath,
            "TMPDIR": str(root),
        }
        first, second = root / "first.json", root / "second.json"
        project = root / "project"
        project.mkdir()
        marker = project / "keep.txt"
        marker.write_text("preserve this work")

        def call(*args, success=True, context=first):
            result = subprocess.run(
                [str(binary), "managed", "--context", str(context), *args],
                env=env, text=True, capture_output=True, timeout=30,
            )
            data = json.loads(result.stdout)
            assert data["ok"] == success, (args, data, result.stderr)
            assert (result.returncode == 0) == success, result.stderr
            return data

        owner = call("init")["data"]["owner"]
        call("init", context=second)
        assert call("list")["data"] == []
        item = call("create", str(project), "--tool", "shell", "--title", "first")["data"]
        session_id = item["id"]
        assert item["owner"] == owner
        assert len(session_id) == 16
        assert call("show", session_id)["data"]["owned"]
        assert call("capture", session_id, "--lines", "0", success=False)["error"]["code"] == "invalid_input"
        assert call("create", str(project), "--tool", "codex", "--title", "invalid",
                    "--xats-name", "worker", success=False)["error"]["code"] == "invalid_input"
        assert not log.exists(), "Create/list/show unexpectedly touched tmux"
        for args in [
            ("start", session_id), ("stop", session_id), ("restart", session_id),
            ("remove", session_id), ("rename", session_id, "--title", "new"),
            ("send", session_id, "message"), ("capture", session_id),
        ]:
            error = call(*args, context=second, success=False)["error"]
            assert error["code"] == "permission_denied", error
        assert not log.exists(), "An unauthorized operation reached tmux"
        assert call("remove", session_id[:8], success=False)["error"]["code"] == "invalid_id"
        adoption = subprocess.run(
            [str(binary), "session", "set-owner", session_id],
            env=env, text=True, capture_output=True, timeout=30,
        )
        assert adoption.returncode == 0, adoption.stderr
        assert json.loads(adoption.stdout)["data"]["owner"] is None
        assert call("remove", session_id, success=False)["error"]["code"] == "permission_denied"
        assert not log.exists(), "Human adoption or rejection reached tmux"
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            created = list(pool.map(
                lambda index: call("create", str(project), "--tool", "shell",
                                   "--title", f"worker-{index}")["data"]["id"],
                range(8),
            ))
        listed = call("list")["data"]
        assert len(listed) == 9 and len(set(created)) == 8
        assert not log.exists(), "Concurrent creation reached tmux"
        call("remove", created[0])
        assert marker.read_text() == "preserve this work"
        assert len(call("list")["data"]) == 8
        assert "kill-session" not in log.read_text()
        assert "kill-server" not in log.read_text()
        print(json.dumps({"status": "PASS", "checks": [
            "context", "create", "JSON", "owner_rejection", "exact_id",
            "human_takeover", "parallel_create", "safe_remove",
            "no_real_tmux", "code_preserved",
        ]}))


if __name__ == "__main__":
    main()
