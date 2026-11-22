import base64
import hashlib
import json
import os
from pathlib import Path
import socketserver
import sqlite3
import struct
import subprocess
import sys
import tempfile
import threading
import uuid


def exact(stream, size):
    data = stream.read(size)
    if len(data) != size:
        raise EOFError('WebSocket peer closed')
    return data


def read_frame(stream):
    first, second = exact(stream, 2)
    size = second & 127
    if size == 126:
        size = struct.unpack('!H', exact(stream, 2))[0]
    elif size == 127:
        size = struct.unpack('!Q', exact(stream, 8))[0]
    assert size <= 65536
    mask = exact(stream, 4) if second & 128 else None
    data = exact(stream, size)
    if mask:
        data = bytes(byte ^ mask[index % 4] for index, byte in enumerate(data))
    return first & 15, data


def write_frame(stream, opcode, data):
    prefix = bytes([128 | opcode, len(data)]) if len(data) < 126 else (
        bytes([128 | opcode, 126]) + struct.pack('!H', len(data)))
    stream.write(prefix + data)
    stream.flush()


class WebSocket(socketserver.StreamRequestHandler):
    def handle(self):
        headers = {}
        exact_line = self.rfile.readline()
        assert exact_line.startswith(b'GET ')
        for line in iter(self.rfile.readline, b'\r\n'):
            name, value = line.decode().split(':', 1)
            headers[name.lower()] = value.strip()
        key = headers['sec-websocket-key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11'
        accept = base64.b64encode(hashlib.sha1(key.encode()).digest()).decode()
        self.wfile.write(('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n'
                          'Connection: Upgrade\r\nSec-WebSocket-Accept: '
                          + accept + '\r\n\r\n').encode())
        self.wfile.flush()
        while True:
            try:
                opcode, data = read_frame(self.rfile)
            except EOFError:
                return
            if opcode == 8:
                write_frame(self.wfile, 8, b'\x03\xe8')
                return
            if opcode == 9:
                write_frame(self.wfile, 10, data)
                continue
            assert opcode == 1
            request = json.loads(data)
            if 'id' not in request:
                continue
            result = self.server.reply(request)
            write_frame(self.wfile, 1, json.dumps({'id': request['id'], **result}).encode())


class Backend(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True

    def __init__(self):
        super().__init__(('127.0.0.1', 0), WebSocket)
        self.threads = {}
        self.calls = []
        self.mode = 'normal'
        self.cwd = None
        self.db = None

    def reply(self, request):
        method, params = request['method'], request.get('params', {})
        self.calls.append((method, params))
        if method == 'initialize':
            return {'result': {'userAgent': 'fixture'}}
        if method == 'config/read':
            return {'result': {'config': {'model_provider': 'fixture'}}}
        if method in {'thread/start', 'thread/fork'}:
            return {'result': self.create(params)}
        if method == 'thread/name/set':
            if self.mode == 'name-error':
                return {'error': {'code': -32000,
                                 'message': 'name failed: key-left Bearer fixture-secret-token'}}
            return {'result': {}}
        if method == 'thread/read':
            return {'result': {'thread': self.threads[params['threadId']]['thread']}}
        raise AssertionError(method)

    def create(self, params):
        thread = str(uuid.uuid4())
        path = self.cwd / f'{thread}.jsonl'
        path.write_text('{}\n')
        config = params.get('config', {})
        response = {
            'thread': {'id': thread, 'cwd': str(self.cwd), 'path': str(path),
                       'status': {'type': 'active'}},
            'model': 'wrong' if self.mode == 'model-error' else params.get('model', 'fixture-model'),
            'modelProvider': params['modelProvider'],
            'approvalPolicy': params.get('approvalPolicy', 'on-request'),
            'approvalsReviewer': params.get('approvalsReviewer', 'user'),
            'sandbox': {'type': {'workspace-write': 'workspaceWrite',
                                'danger-full-access': 'dangerFullAccess'}.get(
                                    params.get('sandbox'), 'workspaceWrite')},
            'reasoningEffort': config.get('model_reasoning_effort', 'medium'),
            'serviceTier': params.get('serviceTier'),
        }
        self.threads[thread] = response
        if self.mode == 'stale-slot':
            with sqlite3.connect(self.db) as connection:
                connection.execute('UPDATE agent_slot SET xats_runtime_generation='
                                   'xats_runtime_generation+1 WHERE slot=0')
        return response


TMUX = '''#!{python}
import json, os, pathlib, sys
root=pathlib.Path(os.environ['FIXTURE_ROOT'])
args=sys.argv[1:]
if args[:1]==['-L']:
    args=args[2:]
with (root/'tmux.jsonl').open('a') as stream:
    stream.write(json.dumps(args)+'\\n')
state=json.loads((root/'state.json').read_text())
if args[0]=='has-session':
    sys.exit(0 if state.get('live',True) else 1)
if args[0]=='display-message' and not state.get('live',True):
    print('cannot find pane',file=sys.stderr)
    sys.exit(1)
if args[0]=='list-panes':
    pane=os.environ.get('TMUX_PANE','%1')
    if args[-1]=='#{{pane_id}} #{{pane_pid}}':
        print(pane,os.getppid())
    elif '|' in args[-1]:
        print(pane+'||'+state['session'])
    else:
        print(pane)
elif args[0]=='display-message':
    print(state.get('primary','%1'))
elif args[0]=='show-option' and args[-1]=='@aoe_agent_pane':
    print(state.get('primary','%1'))
elif args[0]=='new-session':
    state['live']=True
    (root/'state.json').write_text(json.dumps(state))
'''

CODEX = '''#!{python}
import json, os, pathlib, sqlite3, subprocess, sys
root=pathlib.Path(os.environ['FIXTURE_ROOT'])
state=json.loads((root/'state.json').read_text())
with sqlite3.connect(state['db']) as connection:
    thread=connection.execute('SELECT native_session_id FROM agent_slot WHERE slot=?',
                              (int(os.environ['FIXTURE_SLOT']),)).fetchone()[0]
assert sys.argv[1:3]==['resume',thread],sys.argv
assert '--approve-for-me' not in sys.argv
assert '--dangerously-bypass-approvals-and-sandbox' not in sys.argv
check=subprocess.run([sys.executable,'-c',
    "import fcntl,sys; f=open(sys.argv[1],'a+b'); fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)",
    str(pathlib.Path(state['db']).parent/'managed-operation.lock')],capture_output=True)
assert check.returncode==0,'Operation lock leaked through UI exec'
(root/'exec.json').write_text(json.dumps(dict(pid=os.getpid(),args=sys.argv[1:],thread=thread)))
'''


def call(binary, env, *args):
    process = subprocess.Popen([str(binary), '-p', 'default', *args], env=env,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    stdout, stderr = process.communicate(timeout=40)
    return process.returncode, stdout, stderr, process.pid


def fixture(binary, directory, backend):
    directory.mkdir()
    (directory / 'bin').mkdir()
    env = {'HOME': str(directory), 'XDG_CONFIG_HOME': str(directory / 'config'),
           'PATH': str(directory / 'bin') + os.pathsep + os.defpath,
           'TMPDIR': str(directory), 'FIXTURE_ROOT': str(directory),
           'CODEX_HOME': str(directory / '.codex'),
           'CROSS_AGENT_TEAMS_MCP_HOME': str(directory / 'absent-xats')}
    env['CROSS_AGENT_TEAMS_CODEX_WS_URL'] = f'ws://127.0.0.1:{backend.server_address[1]}'
    for name, script in [('tmux', TMUX), ('codex', CODEX), ('ps', '#!{python}\nraise SystemExit(99)\n')]:
        path = directory / 'bin' / name
        path.write_text(script.format(python=sys.executable))
        path.chmod(0o700)
    (directory / 'state.json').write_text(json.dumps({'session': 'uncreated', 'live': False}))
    result = call(binary, env, 'add', str(directory), '-t', 'fixture', '-c', 'codex')
    assert result[0] == 0, result
    sessions_path = next(directory.rglob('sessions.json'))
    records = json.loads(sessions_path.read_text())
    identifier = next(item['id'] for item in records if item['title'] == 'fixture')
    record = next(item for item in records if item['id'] == identifier)
    record['primary_pane']['cross_agent_team'] = True
    record['cross_agent_team'] = True
    record['xats_identity_key'] = 'key-left'
    sessions_path.write_text(json.dumps(records))
    db = next(directory.rglob('aoe.db'))
    state = {'session': f'aoe_fixture_{identifier[:8]}', 'db': str(db), 'live': True}
    (directory / 'state.json').write_text(json.dumps(state))
    with sqlite3.connect(db) as connection:
        for slot in [0, 1]:
            connection.execute('INSERT INTO agent_slot(instance_id,slot,agent,native_session_id,'
                               'cwd,tmux_pane,xats_identity_key,cross_agent_team,last_seen_at) '
                               'VALUES(?,?,\'codex\',\'\',?,?,?,1,1)',
                               (identifier, slot, str(directory), f'%{slot+1}', ['key-left', 'key-right'][slot]))
    backend.cwd, backend.db = directory, db
    return env, identifier, db


def launch(binary, backend, directory, env, identifier, slot=0, thread=None, options=None):
    marker = directory / 'exec.json'
    marker.unlink(missing_ok=True)
    endpoint = f'ws://127.0.0.1:{backend.server_address[1]}'
    launch_id = str(uuid.uuid4())
    context = {**env, 'TMUX_PANE': env.get('TMUX_PANE', f'%{slot+1}'), 'FIXTURE_SLOT': str(slot),
               'XATS_IDENTITY_KEY': ['key-left', 'key-right'][slot],
               'CROSS_AGENT_TEAMS_MCP_TOKEN': 'fixture-secret-token'}
    command = ['codex', '--remote', endpoint, '-C', str(directory), '-c',
               f'xats.agent_id="{launch_id}"', '-c',
               f'shell_environment_policy.set.XATS_CODEX_LAUNCH_ID="{launch_id}"']
    if thread:
        command += ['resume', thread]
    else:
        command += options if options is not None else ['fixture prompt', '--search', '--model=fixture-model',
                    '--config=model_reasoning_effort="high"', '--config=service_tier="fast"',
                    '-c', 'mcp_servers.fixture.http_headers.Authorization="Bearer fixture-secret-token"',
                    '--approve-for-me']
    serialized = subprocess.run([str(binary), '__codex-arguments'], env=context,
                                input=b'\0'.join(word.encode() for word in command)+b'\0',
                                capture_output=True, timeout=20)
    assert serialized.returncode == 0, serialized.stderr
    argument_file = Path(serialized.stdout.decode().strip())
    assert argument_file.stat().st_mode & 0o077 == 0
    assert '--remote' not in [str(binary), '__codex-launch', str(argument_file)]
    result = call(binary, context, '__codex-launch', '--instance-id', identifier,
                  '--launch-id', launch_id, '--endpoint', endpoint,
                  '--working-directory', str(directory), '--command-file', str(argument_file))
    assert not argument_file.exists(), 'Consumed private arguments must be removed'
    diagnostics = result[1] + result[2]
    diagnostics += ''.join(path.read_text() for path in directory.rglob('debug.log'))
    for secret in ['key-left', 'key-right', 'fixture-secret-token']:
        assert secret not in diagnostics, (backend.mode, diagnostics)
    return result, json.loads(marker.read_text()) if marker.exists() else None


def normal_and_resume(binary, backend, root):
    directory = root / 'normal'
    env, identifier, db = fixture(binary, directory, backend)
    left, left_exec = launch(binary, backend, directory, env, identifier)
    assert left[0] == 0, left
    assert left_exec['pid'] == left[3]
    right, right_exec = launch(binary, backend, directory, env, identifier, slot=1)
    assert right[0] == 0, right
    assert left_exec['thread'] != right_exec['thread']
    count = len(backend.threads)
    resumed, resumed_exec = launch(binary, backend, directory, env, identifier,
                                  thread=left_exec['thread'])
    assert resumed[0] == 0, resumed
    assert resumed_exec['thread'] == left_exec['thread']
    assert len(backend.threads) == count
    with sqlite3.connect(db) as connection:
        for slot in [0, 1]:
            row = connection.execute('SELECT native_session_id FROM agent_slot WHERE slot=?',
                                     (slot,)).fetchone()
            assert row[0]
        assert connection.execute("SELECT count(*) FROM events WHERE kind='codex_thread_created'").fetchone()[0] == 2
    assert not (directory / 'absent-xats').exists()


def failed_start_keeps_uuid(binary, backend, root, mode):
    directory = root / mode
    env, identifier, db = fixture(binary, directory, backend)
    backend.mode = mode
    result, executed = launch(binary, backend, directory, env, identifier)
    assert result[0] != 0 and executed is None, result
    with sqlite3.connect(db) as connection:
        native, generation = connection.execute(
            'SELECT native_session_id,xats_runtime_generation FROM agent_slot WHERE slot=0').fetchone()
        failures = connection.execute("SELECT count(*) FROM events WHERE kind='codex_launch_failed'").fetchone()[0]
    if mode == 'stale-slot':
        assert not native and generation == 1
        assert failures == 0, 'Late old error must not poison new owner'
    else:
        assert native and generation == 1 and failures == 1
        backend.mode = 'normal'
        retried, executed = launch(binary, backend, directory, env, identifier, thread=native)
        assert retried[0] == 0 and executed['thread'] == native, retried
    backend.mode = 'normal'


def cold_resume_without_uuid_has_no_side_effects(binary, backend, root):
    directory = root / 'cold-missing-id'
    env, identifier, db = fixture(binary, directory, backend)
    state = json.loads((directory / 'state.json').read_text())
    state['live'] = False
    (directory / 'state.json').write_text(json.dumps(state))
    (directory / 'tmux.jsonl').write_text('')
    calls = len(backend.calls)
    result = call(binary, env, 'session', 'restart', identifier)
    assert result[0] != 0 and 'original conversation' in result[2], result
    events = [json.loads(line) for line in (directory / 'tmux.jsonl').read_text().splitlines()]
    assert not any(event[:1] == ['new-session'] for event in events)
    assert len(backend.calls) == calls
    with sqlite3.connect(db) as connection:
        assert connection.execute('SELECT native_session_id,xats_runtime_generation FROM agent_slot WHERE slot=0').fetchone() == ('', 0)


def stop_start_binds_new_pane_without_discarding_old_proof(binary, backend, root):
    directory = root / 'stop-start'
    env, identifier, db = fixture(binary, directory, backend)
    result, executed = launch(binary, backend, directory, env, identifier)
    assert result[0] == 0, result
    original = executed['thread']
    state = json.loads((directory / 'state.json').read_text())
    state.update(live=False, primary='%3')
    (directory / 'state.json').write_text(json.dumps(state))
    started = call(binary, env, 'session', 'start', identifier)
    assert started[0] == 0, started
    with sqlite3.connect(db) as connection:
        row = connection.execute('SELECT tmux_pane,native_session_id FROM agent_slot WHERE slot=0').fetchone()
        assert row == ('%3', original), row
    result, executed = launch(binary, backend, directory,
                              {**env, 'TMUX_PANE': '%3'}, identifier)
    assert result[0] == 0, result
    assert executed['thread'] != original


def fork_allocates_and_persists_its_own_uuid(binary, backend, root):
    directory = root / 'fork'
    env, identifier, db = fixture(binary, directory, backend)
    parent = str(uuid.uuid4())
    result, executed = launch(binary, backend, directory, env, identifier,
                              options=['fork', parent, '--approve-for-me'])
    assert result[0] == 0, result
    assert executed['thread'] != parent
    assert any(method == 'thread/fork' and params['threadId'] == parent
               for method, params in backend.calls)


def early_failure_consumes_private_arguments(binary, root):
    directory = root / 'early-failure'
    directory.mkdir()
    env = {'HOME': str(directory), 'PATH': os.defpath,
           'XDG_CONFIG_HOME': str(directory / 'config')}
    words = ['codex', '', 'line1\nline2']
    result = subprocess.run([str(binary), '__codex-arguments'], env=env,
                            input=b'\0'.join(word.encode() for word in words)+b'\0',
                            capture_output=True, timeout=20)
    assert result.returncode == 0, result.stderr
    argument_file = Path(result.stdout.decode().strip())
    assert json.loads(argument_file.read_text()) == words
    assert argument_file.stat().st_mode & 0o077 == 0
    result = call(binary, env, '__codex-launch', '--instance-id', uuid.uuid4().hex[:16],
                  '--launch-id', str(uuid.uuid4()), '--endpoint', 'ws://127.0.0.1:1',
                  '--working-directory', str(directory / 'missing'),
                  '--command-file', str(argument_file))
    assert result[0] != 0, result
    assert 'Resolving Codex launch directory' in result[2], result
    assert not argument_file.exists(), 'Early failure left private arguments behind'


def main():
    binary = Path(sys.argv[1]).resolve()
    backend = Backend()
    worker = threading.Thread(target=backend.serve_forever, daemon=True)
    worker.start()
    try:
        with tempfile.TemporaryDirectory(prefix='aoe-codex-launch-', dir='/private/tmp') as temp:
            root = Path(temp)
            normal_and_resume(binary, backend, root)
            for mode in ['model-error', 'name-error', 'stale-slot']:
                failed_start_keeps_uuid(binary, backend, root, mode)
            cold_resume_without_uuid_has_no_side_effects(binary, backend, root)
            stop_start_binds_new_pane_without_discarding_old_proof(binary, backend, root)
            fork_allocates_and_persists_its_own_uuid(binary, backend, root)
            early_failure_consumes_private_arguments(binary, root)
        print(json.dumps({'status': 'PASS', 'scenarios': 10,
                          'real_tmux_calls': 0, 'model_calls': 0, 'xats_registrations': 0}))
    finally:
        backend.shutdown()
        backend.server_close()
        worker.join()


if __name__ == '__main__':
    main()
