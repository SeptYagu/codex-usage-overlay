import test, { after, before } from 'node:test';
import assert from 'node:assert/strict';
import * as fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { request } from 'node:http';
import { randomUUID } from 'node:crypto';
import { apply, deriveProfileId, inject, MAX_BODY_BYTES, name, startBridge, validateRpc } from '../index.js';
import { restrictPath } from '../file-security.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const testBase = path.resolve(here, '..', '.test-tmp');
let suiteDirectory;
const statusPath = '/api/codex-subscription/status';
const usagePath = '/api/codex-subscription/usage/accounts';
const listPath = '/api/session/list';
const promptPath = '/api/session/prompt';
const methodFor = (route) => route.slice('/api/'.length);
const envelope = (route = statusPath, payload = {}) => ({ type: 'client-request', rpcId: randomUUID(), method: methodFor(route), payload });
const prompt = (text = '暂停', sessionId = `session-${randomUUID()}`) => ({ args: { request: {
  requestId: randomUUID(), sessionId, mode: 'steer', content: [{ type: 'text', text }],
} } });

function fakeConnection() {
  const state = { created: [], disposed: 0, calls: [], channels: [], handler: undefined };
  const peer = { principal: { kind: 'owner' }, async dispose() { state.disposed += 1; } };
  return { state, createPeer(options) { state.created.push(options); return peer; },
    createSharedFetchHandler(channel) {
      state.channels.push(channel);
      return { async fetch(req, actualPeer) {
        assert.equal(actualPeer, peer);
        const body = await req.json();
        state.calls.push({ url: req.url, method: req.method, headers: [...req.headers], body, signal: req.signal });
        if (state.handler) return state.handler(req, body);
        return Response.json({ type: 'server-response', rpcId: body.rpcId, result: { ok: true, value: { sample: true } } });
      } };
    } };
}
async function directory() { return fs.mkdtemp(path.join(suiteDirectory, 'case-')); }
async function fixture(t, options = {}) {
  const root = options.discoveryDirectory ?? await directory();
  const connection = options.connection ?? fakeConnection();
  const bridge = await startBridge({ connection, discoveryDirectory: root, ...options });
  t.after(() => bridge.dispose());
  const record = JSON.parse(await fs.readFile(bridge.recordPath, 'utf8'));
  return { bridge, connection, record, root };
}
async function call(record, route = statusPath, body = envelope(route), options = {}) {
  const target = new URL(record.origin);
  const headers = { host: target.host, 'content-type': 'application/json', authorization: `Bearer ${record.token}`, ...options.headers };
  for (const key of Object.keys(headers)) if (headers[key] === undefined) delete headers[key];
  return new Promise((resolve, reject) => {
    const req = request({ host: target.hostname, port: target.port, path: route,
      method: options.method ?? 'POST',
      // Flat raw headers preserve intentional duplicates, including Host, on
      // Node 22+ without the HTTP Agent rejecting an object-array Host first.
      headers: Object.entries(headers).flatMap(([key, value]) =>
        (Array.isArray(value) ? value : [value]).flatMap((item) => [key, item])) }, (res) => {
      let raw = '';
      res.setEncoding('utf8');
      res.on('data', (chunk) => { raw += chunk; });
      res.once('end', () => {
        let value;
        try { value = JSON.parse(raw); } catch { value = undefined; }
        resolve({ status: res.statusCode, headers: res.headers, value, raw });
      });
    });
    req.once('error', reject);
    req.end(options.rawBody ?? JSON.stringify(body));
  });
}

before(async () => {
  await fs.mkdir(testBase, { recursive: true, mode: 0o700 });
  suiteDirectory = await fs.mkdtemp(path.join(testBase, 'suite-'));
});
after(async () => {
  // Delete only the exact fixture subtree minted by this process.
  assert.equal(path.dirname(path.resolve(suiteDirectory)), testBase);
  assert.match(path.basename(suiteDirectory), /^suite-/);
  await fs.rm(suiteDirectory, { recursive: true, force: true });
});

test('package exposes an ESM-only Host plugin and matching bundle patch', async () => {
  assert.equal(name, 'codex-usage-overlay-bridge');
  assert.deepEqual(inject, ['connection']);
  const manifest = JSON.parse(await fs.readFile(path.join(here, '..', 'package.json'), 'utf8'));
  assert.equal(manifest.type, 'module');
  assert.equal(manifest.dsh.bundle.patch, './cordis.patch.yml');
  assert.equal(manifest.dependencies, undefined);
  assert.match(await fs.readFile(path.join(here, '..', 'cordis.patch.yml'), 'utf8'), /name: 'dsh-codex-usage-overlay-bridge'/);
});

test('discovery is private, non-OAuth, unique per instance and owner Peer is interaction-free', async (t) => {
  const root = await directory();
  const a = await fixture(t, { discoveryDirectory: root, profileLabel: 'Desktop' });
  const b = await fixture(t, { discoveryDirectory: root });
  assert.deepEqual(Object.keys(a.record).sort(), ['instanceId', 'origin', 'profileLabel', 'schemaVersion', 'token']);
  assert.equal(a.record.schemaVersion, 1);
  assert.equal(a.record.instanceId, a.bridge.instanceId);
  assert.equal(a.record.profileLabel, 'Desktop');
  assert.match(a.record.token, /^[0-9a-f]{64}$/);
  assert.match(a.record.origin, /^http:\/\/127\.0\.0\.1:[1-9]\d*$/);
  assert.notEqual(a.record.token, b.record.token);
  assert.notEqual(a.record.instanceId, b.record.instanceId);
  assert.notEqual(a.record.origin, b.record.origin);
  assert.deepEqual(a.connection.state.created, [{ principal: { kind: 'owner' }, interactions: 'none' }]);
  assert.deepEqual(a.connection.state.channels, ['/api']);
  // Windows restrictPath has already verified the protected DACL using the real OS.
  if (process.platform !== 'win32') {
    assert.equal((await fs.stat(root)).mode & 0o777, 0o700);
    assert.equal((await fs.stat(a.bridge.recordPath)).mode & 0o777, 0o600);
  }
  await a.bridge.dispose();
  await assert.rejects(fs.stat(a.bridge.recordPath), { code: 'ENOENT' });
  assert.ok(await fs.stat(b.bridge.recordPath));
  assert.equal(a.connection.state.disposed, 1);
  await a.bridge.dispose();
  assert.equal(a.connection.state.disposed, 1);
});

test('public profile identity survives bridge restarts without exposing the profile path', async (t) => {
  const profile = path.resolve(suiteDirectory, 'private-profile-location');
  const a = await fixture(t, { profileDirectory: profile });
  const b = await fixture(t, { profileDirectory: path.join(profile, '.') });
  const c = await fixture(t, { profileDirectory: path.resolve(suiteDirectory, 'other-profile') });
  assert.match(a.record.profileId, /^[a-f0-9]{64}$/);
  assert.equal(a.record.profileId, b.record.profileId);
  assert.notEqual(a.record.profileId, c.record.profileId);
  assert.notEqual(a.record.instanceId, b.record.instanceId);
  assert.notEqual(a.record.token, b.record.token);
  assert.equal(JSON.stringify(a.record).includes(profile), false);
  assert.equal(deriveProfileId(undefined), undefined);
  assert.throws(() => deriveProfileId('relative-profile'), /Invalid DSH profile location/);
  if (process.platform === 'win32') assert.equal(deriveProfileId(profile.toUpperCase()), a.record.profileId);
});

test('only exact local authority and dedicated Bearer without Origin are admitted', async (t) => {
  const { record, connection } = await fixture(t);
  for (const headers of [
    { authorization: undefined }, { authorization: 'Bearer wrong' }, { authorization: `Basic ${record.token}` },
    { authorization: `Bearer  ${record.token}` },
  ]) assert.equal((await call(record, statusPath, envelope(), { headers })).status, 401);
  for (const headers of [
    { origin: record.origin }, { origin: 'null' }, { origin: '' },
    { host: 'example.com' }, { host: '127.0.0.1:1' }, { host: 'localhost:1' },
  ]) assert.equal((await call(record, statusPath, envelope(), { headers })).status, 403);
  assert.equal(connection.state.calls.length, 0);
  const reply = await call(record);
  assert.equal(reply.status, 200);
  assert.equal(reply.headers['cache-control'], 'no-store');
  assert.equal(reply.headers['access-control-allow-origin'], undefined);
  assert.equal(reply.headers['set-cookie'], undefined);
  assert.equal(reply.raw.includes(record.token), false);
});

test('duplicate authorization and host headers cannot bypass admission', async (t) => {
  const { record, connection } = await fixture(t);
  const reply = await call(record, statusPath, envelope(), { headers: { authorization: [`Bearer ${record.token}`, `Bearer ${record.token}`] } });
  assert.equal(reply.status, 401);
  const host = new URL(record.origin).host;
  const duplicateHost = await call(record, statusPath, envelope(), { headers: { host: [host, host] } });
  assert.equal(duplicateHost.status, 403);
  assert.equal(connection.state.calls.length, 0);
});

test('allowlist is exact and reconstructs internal envelopes and response rpcId', async (t) => {
  const { record, connection } = await fixture(t);
  const requests = [
    [statusPath, {}], [usagePath, { force: true }], [listPath, { args: { request: {} } }],
    [promptPath, prompt('Pause')], [promptPath, prompt('暂停')], [promptPath, prompt('暫停', randomUUID())],
  ];
  for (const [route, payload] of requests) {
    const body = envelope(route, payload);
    const reply = await call(record, route, body);
    assert.equal(reply.status, 200);
    assert.equal(reply.value.rpcId, body.rpcId);
    assert.deepEqual(reply.value.result, { ok: true, value: { sample: true } });
    const internal = connection.state.calls.at(-1);
    assert.equal(internal.url, `http://dsh.internal${route}`);
    assert.equal(internal.method, 'POST');
    assert.deepEqual(internal.headers, [['content-type', 'application/json']]);
    assert.equal(internal.body.method, methodFor(route));
    assert.deepEqual(internal.body.payload, payload);
    assert.notEqual(internal.body.rpcId, body.rpcId);
  }
  for (const route of ['/api/session/cancel', '/api/codex-subscription/logout', `${usagePath}?force=true`, `${statusPath}/`,
    '/api/plugins/codex-subscription/status', '/API/session/list', '/api/session/%6cist', '/api/session/prompt/../cancel']) {
    assert.equal((await call(record, route, envelope())).status, 404);
  }
  assert.equal((await call(record, statusPath, envelope(), { method: 'GET' })).status, 405);
  assert.equal(connection.state.calls.length, requests.length);
});

test('strict schemas prevent arbitrary prompt, route confusion and extra argument injection', () => {
  const bad = [
    envelope(statusPath, { force: 'true' }), envelope(statusPath, { accountId: randomUUID() }),
    envelope(usagePath, { force: true, token: 'forbidden' }),
    envelope(listPath, { args: { request: { sessionId: randomUUID() } } }),
    envelope(listPath, { args: { request: {} }, extra: true }),
    { ...envelope(), method: 'session/prompt' }, { ...envelope(), rpcId: '\ninvalid' },
    { ...envelope(), params: {} }, { ...envelope(), type: 'server-response' },
    envelope(promptPath, prompt('do arbitrary work')), envelope(promptPath, prompt('pause')),
    envelope(promptPath, prompt('暂停 ')), envelope(promptPath, prompt('暂停\nrun command')),
    envelope(promptPath, { args: { request: { ...prompt().args.request, mode: 'followup' } } }),
    envelope(promptPath, { args: { request: { ...prompt().args.request, requestId: 'invalid' } } }),
    envelope(promptPath, { args: { request: { ...prompt().args.request, sessionId: 'someone-else' } } }),
    envelope(promptPath, { args: { request: { ...prompt().args.request, extra: true } } }),
    envelope(promptPath, { args: { request: { ...prompt().args.request, content: [{ type: 'image', text: '暂停' }] } } }),
    envelope(promptPath, { args: { request: { ...prompt().args.request, content: [{ type: 'text', text: '暂停', url: 'x' }] } } }),
    envelope(promptPath, { args: { request: { ...prompt().args.request, content: [] } } }),
  ];
  for (const body of bad) assert.equal(validateRpc(`/api/${body.method === 'session/prompt' ? 'session/prompt' : body.method}`, body), undefined);
  assert.equal(validateRpc(statusPath, { ...envelope(), method: 'session/list' }), undefined);
  assert.equal(validateRpc(statusPath, JSON.parse('{"type":"client-request","rpcId":"x","method":"codex-subscription/status","payload":{"__proto__":{}}}')), undefined);
});

test('JSON, content type and bounded chunked/declared request bodies are enforced', async (t) => {
  const { record, connection } = await fixture(t);
  assert.equal((await call(record, statusPath, envelope(), { headers: { 'content-type': 'text/plain' } })).status, 415);
  assert.equal((await call(record, statusPath, envelope(), { rawBody: '{broken' })).status, 400);
  assert.equal((await call(record, statusPath, envelope(), { rawBody: 'x'.repeat(MAX_BODY_BYTES + 1), headers: { 'transfer-encoding': 'chunked' } })).status, 413);
  assert.equal((await call(record, statusPath, envelope(), { rawBody: 'x', headers: { 'content-length': MAX_BODY_BYTES + 1 } })).status, 413);
  assert.equal((await call(record, statusPath, { ...envelope(), extra: true })).status, 400);
  assert.equal(connection.state.calls.length, 0);
});

test('upstream failures and malformed or excessive responses are sanitized', async (t) => {
  const { record, connection } = await fixture(t);
  for (const handler of [
    () => { throw new Error(`secret ${record.token}`); },
    () => new Response(record.token, { status: 500 }),
    (_req, body) => Response.json({ type: 'server-response', rpcId: body.rpcId,
      result: { ok: false, error: { message: record.token, stack: record.token } } }),
    () => Response.json({ type: 'server-response', rpcId: 'wrong', result: { ok: true, value: record.token } }),
    (_req, body) => Response.json({ type: 'server-response', rpcId: body.rpcId, result: { ok: true } }),
    () => new Response('x'.repeat(2 * 1024 * 1024 + 1), { headers: { 'content-type': 'application/json' } }),
  ]) {
    connection.state.handler = handler;
    const reply = await call(record);
    assert.equal(reply.value.result.ok, false);
    assert.equal(reply.raw.includes(record.token), false);
    assert.equal(reply.raw.includes('stack'), false);
    assert.equal(reply.headers['access-control-allow-origin'], undefined);
  }
});

test('request timeout and teardown abort in-flight internal Fetch without hanging', async (t) => {
  const connection = fakeConnection();
  connection.state.handler = () => new Promise(() => {});
  const { record, bridge } = await fixture(t, { connection, requestTimeoutMs: 100 });
  const timed = await call(record);
  assert.equal(timed.status, 504);
  assert.equal(connection.state.calls[0].signal.aborted, true);
  const pending = call(record).catch(() => undefined);
  // Observe fake dispatch, not an arbitrary sleep/poll.
  let invoked;
  const called = new Promise((resolve) => { invoked = resolve; });
  connection.state.handler = () => { invoked(); return new Promise(() => {}); };
  await called;
  await bridge.dispose();
  await pending;
  assert.equal(connection.state.calls.at(-1).signal.aborted, true);
  assert.equal(connection.state.disposed, 1);
  await assert.rejects(fs.stat(bridge.recordPath), { code: 'ENOENT' });
});

test('security failure never publishes discovery and closes its owner Peer', async () => {
  const root = await directory();
  const connection = fakeConnection();
  const securePath = async (target, kind, signal) => {
    if (kind === 'file') throw new Error('private filesystem error');
    return restrictPath(target, kind, signal);
  };
  await assert.rejects(startBridge({ connection, discoveryDirectory: root, securePath }), /Bridge startup failed/);
  assert.deepEqual(await fs.readdir(root), []);
  assert.equal(connection.state.disposed, 1);
});

test('startup cancellation races do not publish discovery or leak a Peer', async () => {
  const root = await directory();
  const connection = fakeConnection();
  const controller = new AbortController();
  let reached;
  const blocked = new Promise((resolve) => { reached = resolve; });
  const securePath = async (_target, _kind, signal) => {
    reached();
    await new Promise((resolve) => signal.addEventListener('abort', resolve, { once: true }));
    signal.throwIfAborted();
  };
  const ready = startBridge({ connection, discoveryDirectory: root, securePath, signal: controller.signal });
  await blocked;
  controller.abort();
  await assert.rejects(ready, /Bridge startup failed/);
  assert.deepEqual(await fs.readdir(root), []);
  assert.equal(connection.state.created.length, 0);
});

test('file publication cancellation closes the live listener and cleans the private temporary file', async () => {
  const root = await directory();
  const connection = fakeConnection();
  const controller = new AbortController();
  let reached;
  const blocked = new Promise((resolve) => { reached = resolve; });
  const securePath = async (target, kind, signal) => {
    await restrictPath(target, kind, signal);
    if (kind === 'file') {
      reached();
      await new Promise((resolve) => signal.addEventListener('abort', resolve, { once: true }));
      signal.throwIfAborted();
    }
  };
  const ready = startBridge({ connection, discoveryDirectory: root, securePath, signal: controller.signal });
  await blocked;
  controller.abort();
  await assert.rejects(ready, /Bridge startup failed/);
  assert.deepEqual(await fs.readdir(root), []);
  assert.equal(connection.state.disposed, 1);
});

test('cleanup never deletes a replacement discovery record', async (t) => {
  const { bridge, record } = await fixture(t);
  const replacement = { ...record, token: 'different-owner' };
  await fs.writeFile(bridge.recordPath, JSON.stringify(replacement));
  await bridge.dispose();
  assert.deepEqual(JSON.parse(await fs.readFile(bridge.recordPath, 'utf8')), replacement);
});

test('symlink/junction directory roots are rejected without following or publishing', async () => {
  const real = await directory();
  const parent = await directory();
  const link = path.join(parent, 'redirect');
  await fs.symlink(real, link, process.platform === 'win32' ? 'junction' : 'dir');
  const connection = fakeConnection();
  await assert.rejects(startBridge({ connection, discoveryDirectory: path.join(link, 'nested') }), /Bridge startup failed/);
  assert.equal(connection.state.created.length, 0);
  assert.deepEqual(await fs.readdir(real), []);
});

test('Cordis registers async cleanup before startup and handles immediate unload', async () => {
  let cleanup;
  const ctx = { connection: fakeConnection(), effect(fn) { cleanup = fn(); } };
  const ready = apply(ctx);
  const closed = cleanup();
  await assert.rejects(ready, /Bridge startup failed/);
  await closed;
  assert.equal(ctx.connection.state.created.length, 0);
  assert.throws(() => apply(ctx, { discoveryDirectory: 'unsafe' }), /Invalid bridge configuration/);
  assert.throws(() => apply(ctx, { profileLabel: 'bad\nlabel' }), /Invalid bridge profile label/);
});
