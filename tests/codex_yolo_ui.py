"""Verify the permissions actually used by the first remote Codex UI turn."""

import fcntl
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time
from types import SimpleNamespace
import http.server
import json
import shutil

from codex_launcher_native import fixture, launch, start, stop, unused_port


class Model(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        self.send_error(503)


def first_turn(codex, arguments, env, directory, thread):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 160, 0, 0))
    process = subprocess.Popen([codex, *arguments, 'test'], env={**env, 'TERM': 'xterm-256color'},
                               cwd=directory, stdin=slave, stdout=slave, stderr=slave,
                               start_new_session=True)
    os.close(slave)
    try:
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline and process.poll() is None:
            if select.select([master], [], [], 0.2)[0]:
                data = os.read(master, 65536)
                if b'\x1b[6n' in data:
                    os.write(master, b'\x1b[1;1R')
            for record in Path(env['CODEX_HOME']).rglob(f'*{thread}*.jsonl'):
                for line in record.read_bytes().splitlines(keepends=True):
                    if line.endswith(b'\n') and b'"turn_context"' in line:
                        item = json.loads(line)
                        if item['type'] == 'turn_context':
                            turn = item['payload']
                            return turn['approval_policy'], turn['sandbox_policy']['type']
        raise AssertionError('No first turn recorded for the target UI thread')
    finally:
        stop(process)
        os.close(master)


def verify(binary, codex, root, model_port):
    backend = SimpleNamespace(server_address=('127.0.0.1', unused_port()), mode='native')
    directory = root/'fixture'
    env, identifier, _ = fixture(binary, directory, backend)
    node = shutil.which('node')
    if node:
        env['PATH'] += os.pathsep + str(Path(node).parent)
    home = Path(env['CODEX_HOME'])
    home.mkdir()
    (home/'config.toml').write_text(
        'model="local-test"\nmodel_provider="local-test"\n'
        f'[projects.{json.dumps(str(directory.resolve()))}]\ntrust_level="trusted"\n'
        '[model_providers.local-test]\nname="fixture"\nwire_api="responses"\n'
        f'base_url="http://127.0.0.1:{model_port}/model"\n')
    server, client = start(codex, env, root, backend.server_address[1])
    try:
        for slot, flag, expected in [(0, '--yolo', ('never', 'danger-full-access')),
                                     (1, '--approve-for-me', ('on-request', 'workspace-write'))]:
            result, ui = launch(binary, backend, directory, env, identifier, slot=slot,
                                options=['--model', 'local-test', flag])
            assert result[0] == 0, result
            actual = first_turn(codex, ui['args'], env, directory, ui['thread'])
            assert actual == expected, (flag, actual, expected)
            print(flag, *actual)
    finally:
        client.close()
        stop(server)


if __name__ == '__main__':
    with http.server.ThreadingHTTPServer(('127.0.0.1', 0), Model) as model:
        threading.Thread(target=model.serve_forever, daemon=True).start()
        try:
            with tempfile.TemporaryDirectory() as temp:
                verify(Path(sys.argv[1]).resolve(), sys.argv[2], Path(temp).resolve(), model.server_port)
        finally:
            model.shutdown()
