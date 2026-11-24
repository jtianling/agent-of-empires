"""Exercise dead-pane deletion with private state and fake tmux processes."""

import http.server
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import threading


TMUX = r'''
import json, os, pathlib, sys
root = pathlib.Path(os.environ['FIXTURE_ROOT'])
args = sys.argv[1:]
assert not os.environ.get('TMUX') and not os.environ.get('TMUX_PANE')
assert args[:1] == ['-L']
args = args[2:]
with (root/'tmux.log').open('a') as output:
    output.write(json.dumps(args)+'\n')
state = json.loads((root/'state.json').read_text())
if args[0] == 'has-session':
    sys.exit(0 if state['live'] else 1)
if args[0] == 'kill-session':
    assert args[-1] == '='+state['session']
    state['live'] = False
    (root/'state.json').write_text(json.dumps(state))
elif args[0] == 'list-panes':
    if '#{socket_path}' in args[-1]:
        state['observations'] += 1
        changed = state['observations'] >= state.get('respawn_at', 999)
        (root/'state.json').write_text(json.dumps(state))
        prefix = '/tmp/private-dead-smoke|123|1700000000|'
        print(prefix+'%1|'+state['session']+'|%1||0|0|0|0|888|/dev/pts/1')
        suffix = '|0|777|/dev/pts/7' if changed else '|1|0|'
        print(prefix+'%2|'+state['session']+'|%1||1|80|0'+suffix)
    elif args[-1] == '#{pane_pid}':
        print('0\n0')
    else:
        print('%1\n%2')
else:
    print('Unexpected fake tmux operation: '+args[0], file=sys.stderr)
    sys.exit(97)
'''


class Backend(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self):
        super().__init__(('127.0.0.1', 0), Handler)
        self.requests = []


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        value = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        self.server.requests.append((self.path, value))
        status, result = response(self.server, self.path, value)
        encoded = json.dumps(result).encode()
        self.send_response(status)
        self.send_header('Content-Length', str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


def response(server, path, value):
    assert path == '/api/runtime/panes/release', (path, value)
    targets = value['targets']
    assert [row['tmux']['pane_id'] for row in targets] == ['%1', '%2']
    assert 'allow_dead_if_identity_unclaimed' not in targets[0]
    assert targets[1]['allow_dead_if_identity_unclaimed'] is True
    assert [row['identity_key'] for row in targets] == ['live-key', 'dead-key']
    mode = server.mode
    if mode in ['managed', 'unsupported', 'unavailable']:
        return (409 if mode == 'managed' else 503 if mode == 'unavailable' else 400), dict(
            ok=False, protocol_version=1, error='managed_control_required' if mode == 'managed' else 'invalid_request',
            detail='refused by fixture', target_index=1, retryable=False)
    agent = dict(agent_id='b9b6aac0-9ac3-42cc-a957-6b158b88d746',
                 name='fixture', team='test', agent_type='codex')
    result = dict(ok=True, protocol_version=1, results=[
        dict(index=0, state='released', agent=agent),
        dict(index=1, state='dead_key_unclaimed', agent=None)])
    if mode == 'wrong-dead-state':
        result['results'][1]['state'] = 'already_unregistered'
    if mode == 'wrong-live-state':
        result['results'][0] = dict(index=0, state='dead_key_unclaimed', agent=None)
    if mode in ['respawn-after', 'respawn-before-cleanup', 'respawn-final']:
        state = json.loads((server.root/'state.json').read_text())
        state['respawn_at'] = {'respawn-after': 3, 'respawn-before-cleanup': 4,
                               'respawn-final': 6}[mode]
        (server.root/'state.json').write_text(json.dumps(state))
    if mode == 'new-key':
        with sqlite3.connect(server.db) as con:
            con.execute("UPDATE agent_slot SET xats_identity_key='new-key' WHERE slot=1")
    return 200, result


def invoke(binary, env, *args):
    return subprocess.run([str(binary), '-p', 'default', *args], env=env,
                          text=True, capture_output=True, timeout=30)


def owned_worktree(root, env):
    repository, worktree = root/'repository', root/'owned-worktree'
    repository.mkdir()
    git_env = {**env, 'GIT_CONFIG_NOSYSTEM': '1'}
    commands = [['init', str(repository)],
                ['-C', str(repository), '-c', 'user.name=Fixture', '-c',
                 'user.email=fixture@example.invalid', 'commit', '--allow-empty', '-m', 'fixture'],
                ['-C', str(repository), 'worktree', 'add', '-b', 'owned', str(worktree)]]
    for args in commands:
        result = subprocess.run(['git', *args], env=git_env, capture_output=True, timeout=20)
        assert result.returncode == 0, result.stderr
    return dict(worktree_path=str(worktree), worktree=dict(
        branch='owned', main_repo_path=str(repository), managed_by_aoe=True,
        cleanup_on_delete=True, created_at='2026-11-15T00:00:00Z'))


def fixture(binary, server, root, mode):
    root.mkdir()
    (root/'bin').mkdir()
    for name, body in [('tmux', TMUX), ('ps', 'raise SystemExit(99)')]:
        path = root/'bin'/name
        path.write_text('#!'+sys.executable+'\n'+body)
        path.chmod(0o700)
    env = dict(HOME=str(root), XDG_CONFIG_HOME=str(root/'config'),
               PATH=str(root/'bin')+os.pathsep+os.defpath, FIXTURE_ROOT=str(root),
               CROSS_AGENT_TEAMS_MCP_HOME=str(root/'xats'),
               CROSS_AGENT_TEAMS_MCP_TOKEN='fixture-token')
    (root/'state.json').write_text(json.dumps(dict(live=True, observations=0)))
    result = invoke(binary, env, 'add', str(root), '-t', 'fixture', '-c', 'codex')
    assert result.returncode == 0, result.stderr
    sessions = next(root.rglob('sessions.json'))
    rows = json.loads(sessions.read_text())
    identifier = rows[0]['id']
    rows[0]['primary_pane']['cross_agent_team'] = True
    rows[0]['cross_agent_team'] = True
    rows[0]['xats_identity_key'] = 'live-key'
    if mode in ['respawn-before-cleanup', 'cleanup-success']:
        rows[0]['primary_pane']['worktree'] = owned_worktree(root, env)
    sessions.write_text(json.dumps(rows))
    db = next(root.rglob('aoe.db'))
    with sqlite3.connect(db) as con:
        for slot, key in enumerate(['live-key', 'dead-key']):
            con.execute('INSERT INTO agent_slot(instance_id,slot,agent,native_session_id,cwd,'
                        'tmux_pane,xats_identity_key,cross_agent_team,last_seen_at) '
                        'VALUES(?,?,\'codex\',\'\',?,?,?,1,1)',
                        (identifier, slot, str(root), f'%{slot+1}', key))
    (root/'state.json').write_text(json.dumps(dict(live=True, observations=0,
                                       session=f'aoe_fixture_{identifier[:8]}')))
    (root/'xats').mkdir()
    (root/'xats/daemon.pid').write_text(json.dumps(dict(pid=os.getpid(), port=server.server_port)))
    server.mode, server.root, server.db = mode, root, db
    server.requests.clear()
    return env, identifier, sessions


def verify(binary, server, root, mode):
    env, identifier, sessions = fixture(binary, server, root/mode, mode)
    args = ['--delete-worktree'] if mode in ['respawn-before-cleanup', 'cleanup-success'] else []
    result = invoke(binary, env, 'remove', identifier, '--keep-container', *args)
    assert len(server.requests) == 1, server.requests
    log = [json.loads(line) for line in (root/mode/'tmux.log').read_text().splitlines()]
    killed = any(row[0] == 'kill-session' for row in log)
    assert not any(row[0] in ['kill-server', 'kill-pane', 'respawn-pane'] for row in log)
    remaining = {row['id'] for row in json.loads(sessions.read_text())}
    if mode in ['success', 'cleanup-success', 'managed', 'unsupported', 'unavailable', 'wrong-dead-state', 'wrong-live-state', 'unavailable']:
        assert result.returncode == 0 and killed and identifier not in remaining, result.stderr
        if mode not in ['success', 'cleanup-success']:
            logs = list((root/mode).rglob('debug.log'))
            assert logs and 'continuing deletion' in logs[0].read_text()
    else:
        assert result.returncode != 0 and not killed and identifier in remaining, result.stderr
        expected = {'managed': 'managed_control_required', 'unsupported': 'invalid_request',
                    'wrong-dead-state': 'Invalid xats pane release state',
                    'wrong-live-state': 'Invalid xats pane release state',
                    'new-key': 'identity metadata changed'}.get(mode, 'panes changed')
        assert expected in result.stderr, result.stderr
    assert (root/mode).is_dir()
    if mode == 'respawn-before-cleanup':
        assert (root/mode/'owned-worktree/.git').is_file(), 'Cleanup ran after pane respawn'
    if mode == 'cleanup-success':
        assert not (root/mode/'owned-worktree').exists(), 'Cleanup positive control did not remove worktree'


def main():
    binary = Path(sys.argv[1]).resolve()
    server = Backend()
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        with tempfile.TemporaryDirectory(prefix='aoe-dead-delete-', dir='/private/tmp') as temp:
            for mode in ['success', 'cleanup-success', 'managed', 'unsupported', 'unavailable', 'wrong-dead-state',
                         'wrong-live-state', 'respawn-after', 'respawn-before-cleanup',
                         'respawn-final', 'new-key']:
                verify(binary, server, Path(temp), mode)
        print(json.dumps(dict(status='PASS', scenarios=11, real_tmux_calls=0,
                              model_calls=0, live_pids_signalled=0)))
    finally:
        server.shutdown()
        server.server_close()
        worker.join()


if __name__ == '__main__':
    main()
