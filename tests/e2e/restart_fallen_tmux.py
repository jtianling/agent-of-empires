#!/usr/bin/python3
import os
import pathlib
import subprocess
import sys
import time

root = pathlib.Path(__GATE__)
socket = __SOCKET__
args = sys.argv[1:]
if args[:1] in (["-S"], ["-L"]):
    args = args[2:]
env = {k: v for k, v in os.environ.items() if k not in ("TMUX", "TMUX_PANE")}
result = subprocess.run([__TMUX__, "-S", socket] + args, env=env,
                        stdout=subprocess.PIPE, stderr=subprocess.PIPE)
if args[:3] == ["list-panes", "-a", "-F"] and "pane_current_command" in args[-1]:
    parent = subprocess.run(["/bin/ps", "-p", str(os.getppid()), "-o", "args="],
                            check=True, stdout=subprocess.PIPE).stdout.decode()
    if (root / "arm").exists() and parent.strip() == __BINARY__:
        rows = [line.split("\t") for line in result.stdout.decode().splitlines()]
        wanted = (root / "arm").read_text().splitlines()
        fallen = {row[2] for row in rows if len(row) == 7 and row[4] in
                  ("sh", "bash", "zsh", "fish", "dash") and row[5] == "0"}
        if set(wanted).issubset(fallen):
            try:
                (root / "claimed").mkdir()
            except FileExistsError:
                pass
            else:
                (root / "sample.txt").write_bytes(result.stdout)
                (root / "parent.txt").write_text(parent)
                (root / "blocked").write_text(str(time.monotonic()))
                deadline = time.monotonic() + 25
                while not (root / "release").exists():
                    if time.monotonic() > deadline:
                        (root / "timeout").write_text("release timed out")
                        break
                    time.sleep(0.02)
                (root / "released").write_text(str(time.monotonic()))
sys.stdout.buffer.write(result.stdout)
sys.stderr.buffer.write(result.stderr)
sys.exit(result.returncode)
