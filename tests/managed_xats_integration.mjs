import { spawn, spawnSync } from 'node:child_process';
import { mkdtemp, mkdir, writeFile, readFile, readdir, rm } from 'node:fs/promises';
import { createServer } from 'node:net';
import { createServer as httpServer } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import assert from 'node:assert/strict';

const [binary, codex, xats] = process.argv.slice(2);
assert.ok(binary && codex && xats, 'Usage: node managed_xats_integration.mjs <aoe> <codex> <xats-cli.js>');
const root = await mkdtemp(join(tmpdir(), 'aoe-managed-integration-'));
const children = [];
const logs = [];
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
let socket;
let model;
let modelCalls = 0;
const env = { PATH: process.env.PATH, HOME: root, CODEX_HOME: join(root, '.codex'),
  CROSS_AGENT_TEAMS_MCP_HOME: join(root, 'xats'), TMPDIR: root };
function launch(command, args) {
  const child = spawn(command, args, { env, cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
  children.push(child);
  const log = { text: '' };
  child.stdout.on('data', data => { log.text += data; });
  child.stderr.on('data', data => { log.text += data; });
  logs.push(log);
  return { child, log };
}
async function port() {
  const server = createServer();
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const chosen = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return chosen;
}
async function findSessions(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isFile() && entry.name === 'sessions.json') return path;
    if (entry.isDirectory()) { const found = await findSessions(path); if (found) return found; }
  }
}
function cli(...args) {
  const result = spawnSync(binary, ['managed', '--context', join(root, 'manager.json'), ...args],
    { env, cwd: root, encoding: 'utf8', timeout: 20000 });
  const parsed = JSON.parse(result.stdout);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(parsed.ok, true);
  return parsed.data;
}
let sequence = 0;
const pending = new Map();
async function rpc(method, params) {
  const id = ++sequence;
  const promise = new Promise((resolve, reject) => {
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`RPC timeout ${method}`)); }, 15000);
    pending.set(id, { resolve: result => { clearTimeout(timer); resolve(result); },
      reject: error => { clearTimeout(timer); reject(error); } });
  });
  socket.send(JSON.stringify({ id, method, params }));
  return promise;
}
async function post(base, operation, body) {
  const response = await fetch(`${base}/api/managed/${operation}`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
    signal: AbortSignal.timeout(15000),
  });
  return { status: response.status, body: await response.json() };
}
try {
  await mkdir(join(root, 'bin'));
  await writeFile(join(root, 'bin/tmux'), '#!/bin/sh\nexit 1\n', { mode: 0o700 });
  env.PATH = join(root, 'bin') + ':' + env.PATH;
  await mkdir(env.CODEX_HOME);
  await mkdir(env.CROSS_AGENT_TEAMS_MCP_HOME);
  const xport = await port();
  const daemon = launch(process.execPath, [xats, 'daemon', '--host', '127.0.0.1', '--port', String(xport),
    '--device', 'managed-e2e', '--db', join(root, 'xats/data.db'),
    '--pid-file', join(root, 'xats/daemon.pid')]);
  for (let i = 0; i < 100 && !daemon.log.text.includes('listening'); i++) {
    if (daemon.child.exitCode !== null) throw new Error(`Daemon exited: ${daemon.log.text}`);
    await sleep(100);
  }
  assert.ok(daemon.log.text.includes('listening'), 'daemon did not listen');
  const base = `http://127.0.0.1:${xport}`;
  await writeFile(join(root, 'xats/daemon.pid'), JSON.stringify({ pid: daemon.child.pid, port: xport }));
  model = httpServer((_req, res) => { modelCalls++; res.writeHead(503).end(); });
  await new Promise(resolve => model.listen(0, '127.0.0.1', resolve));
  await writeFile(join(env.CODEX_HOME, 'config.toml'), `model="local-test"\nmodel_provider="local-test"\n` +
    `[model_providers.local-test]\nname="Local test"\nbase_url="http://127.0.0.1:${model.address().port}/model"\n` +
    `wire_api="responses"\nrequires_openai_auth=false\n[mcp_servers.xats]\nurl="${base}/mcp"\n`);
  const cport = await port();
  const endpoint = `ws://127.0.0.1:${cport}`;
  const runtimeProcess = launch(codex, ['app-server', '--listen', endpoint]);
  for (let i = 0; i < 100; i++) {
    try {
      socket = new WebSocket(endpoint);
      await new Promise((resolve, reject) => {
        socket.addEventListener('open', resolve, { once: true });
        socket.addEventListener('error', reject, { once: true });
      });
      break;
    } catch { await sleep(100); }
  }
  assert.equal(socket.readyState, WebSocket.OPEN, runtimeProcess.log.text);
  socket.addEventListener('message', event => {
    const message = JSON.parse(event.data);
    const handler = pending.get(message.id);
    if (!handler) return;
    pending.delete(message.id);
    if (message.error) handler.reject(new Error(JSON.stringify(message.error)));
    else handler.resolve(message.result);
  });
  await rpc('initialize', { clientInfo: { name: 'aoe-managed-integration', version: '1' },
    capabilities: { experimentalApi: true } });
  socket.send(JSON.stringify({ method: 'initialized' }));
  cli('init');
  const item = cli('create', root, '--tool', 'codex', '--title', 'integration',
    '--xats-name', 'worker', '--xats-team', 'isolated');
  assert.equal(item.xats.state, 'prepared');
  const sessionsPath = await findSessions(root);
  const [record] = JSON.parse(await readFile(sessionsPath, 'utf8'));
  const identity = record.managed_xats;
  const request = { protocol_version: 1, identity_key: record.xats_identity_key,
    fence: identity.claim.fence };
  const thread = await rpc('thread/start', { cwd: root, config: {
    'mcp_servers.xats.http_headers': { 'X-Xats-Managed-Token': identity.claim.mcp_token },
  } });
  await rpc('thread/name/set', { threadId: thread.thread.id, name: 'integration' });
  const runtime = { agent_type: 'codex', thread_id: thread.thread.id, ws_url: endpoint };
  const bound = await post(base, 'bind', { ...request, runtime });
  assert.equal(bound.body.ok, true, JSON.stringify(bound.body));
  let state;
  for (let i = 0; i < 30; i++) {
    state = cli('show', item.id).xats;
    if (state.runtime_ready) break;
    await sleep(200);
  }
  assert.equal(state.runtime_ready, true, JSON.stringify(state));
  const next = await post(base, 'prepare', { protocol_version: 1,
    identity_key: record.xats_identity_key, name: 'worker', team: 'isolated',
    agent_type: 'codex', instance_id: randomUUID(), previous: request.fence });
  assert.equal(next.body.ok, true, JSON.stringify(next.body));
  const stale = await post(base, 'release', request);
  assert.equal(stale.body.error, 'stale_fence');
  const release = await post(base, 'release', { ...request, fence: next.body.fence });
  assert.equal(release.body.state, 'released');
  assert.equal(modelCalls, 0, 'test unexpectedly contacted model endpoint');
  console.log(JSON.stringify({ status: 'PASS', checks: [
    'real_daemon_prepare', 'exact_codex_bind', 'per_instance_mcp_ready',
    'aoe_status', 'restart_cas', 'stale_release_rejected', 'exact_release', 'no_model_turn', 'no_tmux',
  ] }));
} finally {
  socket?.close();
  for (const child of children.reverse()) {
    child.kill('SIGTERM');
    await Promise.race([new Promise(resolve => child.once('exit', resolve)), sleep(3000)]);
    if (child.exitCode === null && child.signalCode === null) {
      child.kill('SIGKILL');
      await new Promise(resolve => child.once('exit', resolve));
    }
  }
  if (model) await new Promise(resolve => model.close(resolve));
  await rm(root, { recursive: true, force: true });
}
