import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


TMUX_STUB = r'''
import json
import os
from pathlib import Path
import sys

root = Path(os.environ["HOME"])
args = sys.argv[1:]
if args[:1] == ["-L"]:
    args = args[2:]
with (root / "calls.jsonl").open("a") as output:
    output.write(json.dumps(args) + "\n")
command = args[0]
buffers_path = root / "buffers.json"
buffers = json.loads(buffers_path.read_text())
failure = os.environ.get("SEND_FAIL", "")
if command == failure or failure == "cleanup" and command in (
    "paste-buffer", "delete-buffer"
):
    print("injected " + command + " failure", file=sys.stderr)
    sys.exit(1)
if command == "show-option":
    print("%42")
elif command == "display-message":
    session = "aoe_sender_" + os.environ["SEND_SESSION_ID"][:8]
    print(session + "|%42")
elif command == "list-panes":
    print("%42|0|0|0|" + os.environ["SEND_SESSION_ID"])
elif command == "load-buffer":
    buffers[args[args.index("-b") + 1]] = sys.stdin.buffer.read().hex()
elif command == "paste-buffer":
    name = args[args.index("-b") + 1]
    (root / "payload").write_bytes(bytes.fromhex(buffers[name]))
    if "-d" in args:
        del buffers[name]
elif command == "delete-buffer":
    buffers.pop(args[args.index("-b") + 1], None)
elif command not in (
    "has-session", "paste-buffer", "delete-buffer", "send-keys"
):
    print("Unexpected tmux command: " + command, file=sys.stderr)
    sys.exit(2)
buffers_path.write_text(json.dumps(buffers))
'''


def main():
    binary = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory(prefix="aoe-send-smoke-") as directory:
        root = Path(directory)
        bin_dir = root / "bin"
        bin_dir.mkdir()
        stub = bin_dir / "tmux"
        stub.write_text("#!" + sys.executable + "\n" + TMUX_STUB)
        stub.chmod(0o700)
        buffer_state = root / "buffers.json"
        preserved = {"existing-buffer": "untouched".encode().hex()}
        buffer_state.write_text(json.dumps(preserved))
        env = {
            "HOME": str(root), "XDG_CONFIG_HOME": str(root / "config"),
            "PATH": str(bin_dir) + os.pathsep + os.defpath,
            "TMPDIR": str(root),
        }
        context = root / "manager.json"
        prefix = [str(binary), "-p", "send-smoke"]
        managed = ["managed", "--context", str(context)]

        def call(args, failure="", success=True):
            result = subprocess.run(
                prefix + args, env={**env, "SEND_FAIL": failure},
                text=True, capture_output=True, timeout=20,
            )
            assert (result.returncode == 0) == success, (
                args, result.stdout, result.stderr,
            )
            return result

        call(managed + ["init"])
        created = call(managed + [
            "create", str(root), "--tool", "shell", "--title", "sender",
        ])
        session_id = json.loads(created.stdout)["data"]["id"]
        env["SEND_SESSION_ID"] = session_id
        storage = next(root.rglob("sessions.json"))
        records = json.loads(storage.read_text())
        records[0]["managed_primary_marked"] = True
        storage.write_text(json.dumps(records))
        routes = [
            ["send", session_id],
            managed + ["send", session_id],
            managed + ["pane", "send", session_id, session_id],
        ]
        calls_path = root / "calls.jsonl"

        def calls():
            return [json.loads(line) for line in calls_path.read_text().splitlines()]

        buffers = set()
        for route in routes:
            for payload in ["/status", "Enter Up C-c", "第一行\n\n末行\n", "-n"]:
                calls_path.write_text("")
                call(route + ["--", payload])
                commands = calls()
                paste = [args for args in commands if args[0] == "paste-buffer"]
                assert len(paste) == 1, commands
                paste = paste[0]
                assert "-p" in paste or any(
                    arg.startswith("-") and "p" in arg for arg in paste[1:]
                ), paste
                assert "-r" in paste or any(
                    arg.startswith("-") and "r" in arg for arg in paste[1:]
                ), paste
                assert (root / "payload").read_bytes() == payload.encode(), commands
                buffer_name = paste[paste.index("-b") + 1]
                loader = next(args for args in commands if args[0] == "load-buffer")
                assert loader[loader.index("-b") + 1] == buffer_name, commands
                assert "-d" in paste, commands
                assert buffer_name not in buffers, commands
                buffers.add(buffer_name)
                assert json.loads(buffer_state.read_text()) == preserved
                target = paste[paste.index("-t") + 1]
                assert target == "%42" or target.startswith("=aoe_sender_"), target
                assert commands[-1] == ["send-keys", "-t", target, "Enter"], commands

            for failure in ["load-buffer", "paste-buffer", "cleanup", "send-keys"]:
                calls_path.write_text("")
                result = call(route + ["sensitive-payload"], failure, success=False)
                commands = calls()
                detail = result.stdout + result.stderr
                assert "injected " in detail, detail
                assert "sensitive-payload" not in detail, detail
                if failure != "send-keys":
                    assert not any(args[0] == "send-keys" for args in commands), commands
                if failure in ("paste-buffer", "cleanup"):
                    loader = next(args for args in commands if args[0] == "load-buffer")
                    cleanup = next(args for args in commands if args[0] == "delete-buffer")
                    assert cleanup[cleanup.index("-b") + 1] == loader[2], commands
                if failure == "cleanup":
                    assert "injected paste-buffer failure" in detail, detail
                    assert "injected delete-buffer failure" in detail, detail
                    assert len(json.loads(buffer_state.read_text())) == 2
                    buffer_state.write_text(json.dumps(preserved))
                else:
                    assert json.loads(buffer_state.read_text()) == preserved

        other = root / "other.json"
        call(["managed", "--context", str(other), "init"])
        calls_path.write_text("")
        for suffix in (["send", session_id, "blocked"],
                       ["pane", "send", session_id, session_id, "blocked"]):
            call(["managed", "--context", str(other), *suffix], success=False)
        assert calls_path.read_text() == "", "Unauthorized send reached tmux"
        print(json.dumps({"status": "PASS", "routes": 3,
                          "payloads": 4, "failure_modes": 4,
                          "real_tmux": False}))


if __name__ == "__main__":
    main()
