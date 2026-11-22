import base64
import http.server
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
from types import SimpleNamespace

from codex_launcher_smoke import exact, fixture, launch, read_frame


def send(stream, value):
    data = json.dumps(value).encode()
    mask = os.urandom(4)
    header = bytes([129, 128 | len(data)]) if len(data) < 126 else (
        bytes([129, 254]) + struct.pack('!H', len(data)))
    encoded = bytes(byte ^ mask[index % 4] for index, byte in enumerate(data))
    stream.write(header + mask + encoded)
    stream.flush()


class Client:
    def __init__(self, port):
        self.socket = socket.create_connection(('127.0.0.1', port), timeout=5)
        self.stream = self.socket.makefile('rwb')
        key = base64.b64encode(os.urandom(16)).decode()
        self.stream.write((f'GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\n'
                           'Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\n'
                           f'Sec-WebSocket-Key: {key}\r\n\r\n').encode())
        self.stream.flush()
        assert self.stream.readline().startswith(b'HTTP/1.1 101')
        while self.stream.readline() != b'\r\n':
            pass
        self.sequence = 0
        self.call('initialize', {'clientInfo': {'name': 'aoe-native-regression', 'version': '1'},
                                 'capabilities': {'experimentalApi': True}})
        send(self.stream, {'method': 'initialized'})

    def call(self, method, params):
        self.sequence += 1
        send(self.stream, {'id': self.sequence, 'method': method, 'params': params})
        while True:
            opcode, data = read_frame(self.stream)
            assert opcode == 1, (opcode, data)
            response = json.loads(data)
            if response.get('id') == self.sequence:
                assert 'error' not in response, response
                return response['result']

    def close(self):
        self.stream.close()
        self.socket.close()


def unused_port():
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        return listener.getsockname()[1]


def start(codex, env, root, port):
    log = (root / 'native.log').open('ab')
    process = subprocess.Popen([codex, 'app-server', '--listen', f'ws://127.0.0.1:{port}'],
                               env=env, cwd=root, stdout=log, stderr=log,
                               start_new_session=True)
    log.close()
    try:
        for _ in range(100):
            if process.poll() is not None:
                raise AssertionError((root / 'native.log').read_text()[-4000:])
            try:
                return process, Client(port)
            except (OSError, EOFError):
                time.sleep(0.05)
        raise AssertionError('Isolated Codex app-server did not become ready')
    except BaseException:
        stop(process)
        raise


def stop(process):
    if process.poll() is not None:
        return
    assert process.pid > 1 and os.getpgid(process.pid) == process.pid
    os.killpg(process.pid, signal.SIGTERM)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        assert os.getpgid(process.pid) == process.pid
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=5)


def profile_thread(binary, backend, directory, env, identifier):
    home = Path(env['CODEX_HOME'])
    (home / 'fixture.config.toml').write_text(
        'model="local-profile"\nmodel_reasoning_effort="high"\nservice_tier="default"\n')
    result, executed = launch(binary, backend, directory, env, identifier,
                              options=['--profile=fixture', '--search', '--approve-for-me'])
    assert result[0] == 0, result
    return executed['thread']


def verify_reloaded(client, threads):
    for thread, model in threads:
        restored = client.call('thread/resume', {'threadId': thread, 'excludeTurns': True,
                                                'model': model})
        assert restored['thread']['id'] == thread
        assert restored['modelProvider'] == 'local-test'
        assert restored['model'] == model


def verify(binary, codex, root, model_port):
    port = unused_port()
    backend = SimpleNamespace(server_address=('127.0.0.1', port), mode='native')
    env, identifier, _db = fixture(binary, root / 'home', backend)
    node = shutil.which('node')
    if node:
        env['PATH'] += os.pathsep + str(Path(node).parent)
    home = Path(env['CODEX_HOME'])
    home.mkdir()
    (home / 'config.toml').write_text(
        'model="local-test"\nmodel_provider="local-test"\n'
        '[model_providers.local-test]\nname="Local test"\n'
        f'base_url="http://127.0.0.1:{model_port}/model"\n'
        'wire_api="responses"\nrequires_openai_auth=false\n')
    process, client = start(codex, env, root, port)
    try:
        result, executed = launch(binary, backend, root / 'home', env, identifier,
                                  options=['--model', 'local-test', '--approve-for-me'])
        assert result[0] == 0, result
        thread = executed['thread']
        metadata = client.call('thread/read', {'threadId': thread, 'includeTurns': False})
        assert metadata['thread']['id'] == thread
        path = Path(metadata['thread']['path'])
        assert path.is_file() and path.stat().st_size > 0
        profiled = profile_thread(binary, backend, root / 'home', env, identifier)
        client.close()
        stop(process)
        process, client = start(codex, env, root, port)
        verify_reloaded(client, [(thread, 'local-test'), (profiled, 'local-profile')])
    finally:
        client.close()
        stop(process)


def main():
    binary = Path(sys.argv[1]).resolve()
    codex = str(Path(sys.argv[2]).resolve())
    requests = []

    class Model(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_args):
            return

        def do_GET(self):
            requests.append(('GET', self.path))
            self.send_response(503)
            self.end_headers()

        def do_POST(self):
            requests.append(('POST', self.path))
            self.send_response(503)
            self.end_headers()

    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Model)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        with tempfile.TemporaryDirectory(prefix='aoe-codex-native-', dir='/private/tmp') as temp:
            verify(binary, codex, Path(temp), server.server_address[1])
        assert not any(method == 'POST' for method, _path in requests), requests
        print(json.dumps({'status': 'PASS', 'checks': ['uuid_before_ui',
            'empty_thread_durable_history', 'exact_uuid_after_isolated_server_restart'],
            'real_tmux_calls': 0, 'model_calls': 0}))
    finally:
        server.shutdown()
        server.server_close()
        worker.join()


if __name__ == '__main__':
    main()
