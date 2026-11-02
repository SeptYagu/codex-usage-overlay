import { createServer } from 'node:http';
import path from 'node:path';
import { randomBytes, randomUUID, timingSafeEqual } from 'node:crypto';
import { prepareDirectory, publishDiscovery, removeOwnedDiscovery, restrictPath } from './file-security.js';

export const name = 'codex-usage-overlay-bridge';
export const inject = ['connection'];
export const MAX_BODY_BYTES = 32 * 1024;
const MAX_RESPONSE_BYTES = 2 * 1024 * 1024;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const SESSION = /^(?:session-)?[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const PAUSE_TEXTS = new Set(['Pause', '暂停', '暫停']);
const recordObject = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);
const keysAre = (value, required, optional = []) => recordObject(value) &&
  required.every((key) => Object.hasOwn(value, key)) &&
  Object.keys(value).every((key) => required.includes(key) || optional.includes(key));

const routes = new Map([
  ['/api/codex-subscription/status', { method: 'codex-subscription/status', payload: subscriptionPayload }],
  ['/api/codex-subscription/usage/accounts', { method: 'codex-subscription/usage/accounts', payload: subscriptionPayload }],
  ['/api/session/list', { method: 'session/list', payload: listPayload }],
  ['/api/session/prompt', { method: 'session/prompt', payload: pausePayload }],
]);

function subscriptionPayload(payload) {
  if (!keysAre(payload, [], ['force']) || (Object.hasOwn(payload, 'force') && typeof payload.force !== 'boolean')) return;
  return Object.hasOwn(payload, 'force') ? { force: payload.force } : {};
}
function listPayload(payload) {
  if (!keysAre(payload, ['args']) || !keysAre(payload.args, ['request']) || !keysAre(payload.args.request, [])) return;
  return { args: { request: {} } };
}
function pausePayload(payload) {
  if (!keysAre(payload, ['args']) || !keysAre(payload.args, ['request'])) return;
  const request = payload.args.request;
  if (!keysAre(request, ['requestId', 'sessionId', 'mode', 'content']) ||
      typeof request.requestId !== 'string' || !UUID.test(request.requestId) ||
      typeof request.sessionId !== 'string' || !SESSION.test(request.sessionId) || request.mode !== 'steer' ||
      !Array.isArray(request.content) || request.content.length !== 1) return;
  const content = request.content[0];
  if (!keysAre(content, ['type', 'text']) || content.type !== 'text' || !PAUSE_TEXTS.has(content.text)) return;
  return { args: { request: { requestId: request.requestId, sessionId: request.sessionId,
    mode: 'steer', content: [{ type: 'text', text: content.text }] } } };
}

export function validateRpc(routePath, envelope) {
  const route = routes.get(routePath);
  if (!route || !keysAre(envelope, ['type', 'rpcId', 'method', 'payload']) ||
      envelope.type !== 'client-request' || envelope.method !== route.method ||
      typeof envelope.rpcId !== 'string' || !/^[A-Za-z0-9_.:-]{1,128}$/.test(envelope.rpcId)) return;
  const payload = route.payload(envelope.payload);
  if (payload === undefined) return;
  // Never forward the caller's route/method/rpcId/object graph to an owner Peer.
  return { method: route.method, payload, externalRpcId: envelope.rpcId };
}

function failure(status, code, message) {
  return Object.assign(new Error(message), { status, code });
}
function abortable(promise, signal) {
  if (signal.aborted) return Promise.reject(failure(503, 'unavailable', 'Bridge unavailable'));
  return new Promise((resolve, reject) => {
    const abort = () => reject(failure(503, 'unavailable', 'Bridge unavailable'));
    signal.addEventListener('abort', abort, { once: true });
    Promise.resolve(promise).then(resolve, reject).finally(() => signal.removeEventListener('abort', abort));
  });
}
function headerCount(request, key) {
  let count = 0;
  for (let i = 0; i < request.rawHeaders.length; i += 2) if (request.rawHeaders[i].toLowerCase() === key) count += 1;
  return count;
}
function authorized(request, token) {
  if (headerCount(request, 'authorization') !== 1) return false;
  const value = request.headers.authorization;
  if (typeof value !== 'string' || !value.startsWith('Bearer ')) return false;
  const supplied = Buffer.from(value.slice(7), 'utf8');
  const expected = Buffer.from(token, 'utf8');
  return supplied.length === expected.length && timingSafeEqual(supplied, expected);
}
function readBody(request, signal) {
  return new Promise((resolve, reject) => {
    let count = 0;
    const chunks = [];
    const finish = (error) => {
      request.off('data', data); request.off('end', end); request.off('error', errorHandler); request.off('aborted', aborted);
      signal.removeEventListener('abort', aborted);
      if (error) { request.resume(); reject(error); }
      else resolve(Buffer.concat(chunks, count));
    };
    const data = (chunk) => {
      count += chunk.length;
      if (count > MAX_BODY_BYTES) finish(failure(413, 'too-large', 'Request too large'));
      else chunks.push(chunk);
    };
    const end = () => finish();
    const errorHandler = () => finish(failure(400, 'invalid-request', 'Invalid request'));
    const aborted = () => finish(failure(408, 'timeout', 'Request interrupted'));
    request.on('data', data); request.once('end', end); request.once('error', errorHandler); request.once('aborted', aborted);
    signal.addEventListener('abort', aborted, { once: true });
    if (signal.aborted) aborted();
  });
}
async function boundedJson(response, signal) {
  if (!(response instanceof Response) || !response.ok ||
      response.headers.get('content-type')?.split(';', 1)[0].trim().toLowerCase() !== 'application/json') {
    await response?.body?.cancel().catch(() => {});
    throw failure(502, 'upstream', 'DSH request failed');
  }
  const reader = response.body?.getReader();
  if (!reader) throw failure(502, 'upstream', 'DSH request failed');
  const chunks = [];
  let count = 0;
  try {
    for (;;) {
      const { value, done } = await abortable(reader.read(), signal);
      if (done) break;
      count += value.byteLength;
      if (count > MAX_RESPONSE_BYTES) throw failure(502, 'upstream', 'DSH response too large');
      chunks.push(Buffer.from(value));
    }
    return JSON.parse(Buffer.concat(chunks, count).toString('utf8'));
  } finally { await reader.cancel().catch(() => {}); reader.releaseLock(); }
}
function send(response, status, value) {
  if (response.destroyed || response.writableEnded) return;
  response.writeHead(status, { 'content-type': 'application/json; charset=utf-8',
    'cache-control': 'no-store', pragma: 'no-cache', 'x-content-type-options': 'nosniff', connection: 'close' });
  response.end(JSON.stringify(value));
}
function errorEnvelope(rpcId, code, message) {
  return { type: 'server-response', rpcId, result: { ok: false, error: { code, message, details: { issues: [] } } } };
}
function validateLabel(label) {
  if (label !== undefined && (typeof label !== 'string' || label.length < 1 || label.length > 80 || /[\u0000-\u001f\u007f]/.test(label))) {
    throw new Error('Invalid bridge profile label');
  }
}
export function defaultDiscoveryDirectory() {
  if (process.platform === 'win32') {
    if (!process.env.LOCALAPPDATA || !path.isAbsolute(process.env.LOCALAPPDATA)) throw new Error('LOCALAPPDATA is unavailable');
    return path.join(process.env.LOCALAPPDATA, 'CodexUsageOverlay', 'dsh-bridges');
  }
  if (!process.env.HOME || !path.isAbsolute(process.env.HOME)) throw new Error('HOME is unavailable');
  return path.join(process.env.HOME, '.local', 'state', 'CodexUsageOverlay', 'dsh-bridges');
}
async function listen(server, signal) {
  await new Promise((resolve, reject) => {
    const cleanup = () => { server.off('error', failed); server.off('listening', ready); signal.removeEventListener('abort', aborted); };
    const failed = () => { cleanup(); reject(new Error('Bridge listener could not start')); };
    const ready = () => { cleanup(); resolve(); };
    const aborted = () => { cleanup(); reject(new Error('Bridge startup cancelled')); };
    server.once('error', failed); server.once('listening', ready);
    signal.addEventListener('abort', aborted, { once: true });
    if (signal.aborted) { aborted(); return; }
    server.listen({ port: 0, host: '127.0.0.1', signal });
  });
}
async function closeServer(server) {
  if (!server) return;
  await new Promise((resolve) => {
    server.close(() => resolve());
    server.closeAllConnections();
  });
}

/** Public only for embedding/tests. Production plugin never accepts filesystem or security overrides. */
export async function startBridge({ connection, discoveryDirectory = defaultDiscoveryDirectory(), profileLabel,
  signal, requestTimeoutMs = 15_000, securePath = restrictPath } = {}) {
  if (!connection || typeof connection.createPeer !== 'function' || typeof connection.createSharedFetchHandler !== 'function') {
    throw new Error('DSH connection service is unavailable');
  }
  validateLabel(profileLabel);
  if (!Number.isInteger(requestTimeoutMs) || requestTimeoutMs < 1 || requestTimeoutMs > 60_000) throw new Error('Invalid bridge timeout');
  const controller = new AbortController();
  const instanceId = randomUUID();
  const token = randomBytes(32).toString('hex');
  let root, peer, server, record, recordPath, disposePromise;
  let started = false;
  let active = 0;
  const dispose = () => {
    if (disposePromise) return disposePromise;
    disposePromise = (async () => {
      controller.abort();
      signal?.removeEventListener('abort', externalAbort);
      await closeServer(server);
      let peerError = false;
      try { await peer?.dispose(); } catch { peerError = true; }
      if (root && record) await removeOwnedDiscovery(root, record);
      if (peerError) throw new Error('Bridge cleanup failed');
    })();
    // Teardown also runs via an event; never leak an unhandled rejection.
    disposePromise.catch(() => {});
    return disposePromise;
  };
  const externalAbort = () => { controller.abort(); if (started) void dispose(); };
  signal?.addEventListener('abort', externalAbort, { once: true });
  if (signal?.aborted) externalAbort();
  try {
    root = await prepareDirectory(discoveryDirectory, controller.signal, securePath);
    controller.signal.throwIfAborted();
    peer = connection.createPeer({ principal: { kind: 'owner' }, interactions: 'none' });
    if (!peer || typeof peer.dispose !== 'function') throw new Error('DSH owner Peer is unavailable');
    const shared = connection.createSharedFetchHandler('/api');
    if (!shared || typeof shared.fetch !== 'function') throw new Error('DSH shared Fetch handler is unavailable');
    server = createServer({ maxHeaderSize: 8 * 1024, requireHostHeader: true }, (request, response) => {
      void handle(request, response).catch(() => send(response, 500, errorEnvelope('bridge-error', 'internal', 'Bridge request failed')));
    });
    server.headersTimeout = 5_000;
    server.requestTimeout = requestTimeoutMs;
    server.keepAliveTimeout = 1_000;
    server.maxConnections = 32;
    server.maxRequestsPerSocket = 100;
    server.on('error', () => { if (started) void dispose(); });
    server.on('clientError', (_error, socket) => { socket.destroy(); });
    await listen(server, controller.signal);
    const address = server.address();
    if (!address || typeof address === 'string' || address.address !== '127.0.0.1') throw new Error('Invalid bridge listener');
    const origin = `http://127.0.0.1:${address.port}`;
    record = { schemaVersion: 1, instanceId, origin, token, ...(profileLabel === undefined ? {} : { profileLabel }) };
    recordPath = await publishDiscovery(root, record, controller.signal, securePath);
    controller.signal.throwIfAborted();
    started = true;
    return Object.freeze({ instanceId, origin, recordPath, dispose });

    async function handle(request, response) {
      let externalRpcId = 'bridge-error';
      const local = new AbortController();
      const stop = () => local.abort();
      const timeout = setTimeout(stop, requestTimeoutMs);
      timeout.unref();
      controller.signal.addEventListener('abort', stop, { once: true });
      const disconnected = () => { if (!response.writableEnded) stop(); };
      response.once('close', disconnected);
      let admitted = false;
      try {
        if (!started || controller.signal.aborted) throw failure(503, 'unavailable', 'Bridge unavailable');
        if (request.socket.remoteAddress !== '127.0.0.1' || headerCount(request, 'host') !== 1 || request.headers.host !== new URL(origin).host) {
          throw failure(403, 'forbidden', 'Invalid local authority');
        }
        if (headerCount(request, 'origin') !== 0) throw failure(403, 'forbidden', 'Browser origins are not permitted');
        if (!authorized(request, token)) throw failure(401, 'unauthorized', 'Bridge authentication required');
        if (request.method !== 'POST') throw failure(405, 'invalid-request', 'Only POST is permitted');
        if (!routes.has(request.url)) throw failure(404, 'not-found', 'Unknown bridge route');
        if (headerCount(request, 'content-type') !== 1 ||
            request.headers['content-type']?.split(';', 1)[0].trim().toLowerCase() !== 'application/json') {
          throw failure(415, 'invalid-request', 'Content type must be application/json');
        }
        if (Number(request.headers['content-length'] ?? 0) > MAX_BODY_BYTES) throw failure(413, 'too-large', 'Request too large');
        if (active >= 16) throw failure(429, 'unavailable', 'Bridge is busy');
        admitted = true; active += 1;
        let envelope;
        try { envelope = JSON.parse((await readBody(request, local.signal)).toString('utf8')); }
        catch (error) { if (error.status) throw error; throw failure(400, 'invalid-request', 'Invalid JSON'); }
        const valid = validateRpc(request.url, envelope);
        if (!valid) throw failure(400, 'invalid-request', 'Invalid bridge RPC');
        externalRpcId = valid.externalRpcId;
        const internalRpcId = randomUUID();
        const upstream = new Request(`http://dsh.internal${request.url}`, { method: 'POST',
          headers: { 'content-type': 'application/json' }, signal: local.signal,
          body: JSON.stringify({ type: 'client-request', rpcId: internalRpcId, method: valid.method, payload: valid.payload }) });
        let result;
        try {
          result = await boundedJson(await abortable(shared.fetch(upstream, peer), local.signal), local.signal);
        } catch { throw failure(local.signal.aborted ? 504 : 502, 'upstream', 'DSH request failed'); }
        if (!keysAre(result, ['type', 'rpcId', 'result']) || result.type !== 'server-response' || result.rpcId !== internalRpcId ||
            !recordObject(result.result) || typeof result.result.ok !== 'boolean') throw failure(502, 'upstream', 'Invalid DSH response');
        if (!result.result.ok) send(response, 200, errorEnvelope(externalRpcId, 'upstream', 'DSH request failed'));
        else if (!Object.hasOwn(result.result, 'value')) throw failure(502, 'upstream', 'Invalid DSH response');
        else send(response, 200, { type: 'server-response', rpcId: externalRpcId, result: { ok: true, value: result.result.value } });
      } catch (error) {
        // No exception text, upstream details, authentication header or token is echoed.
        const status = Number.isInteger(error?.status) ? error.status : 500;
        const code = typeof error?.code === 'string' && error.status ? error.code : 'internal';
        const message = error.status ? error.message : 'Bridge request failed';
        send(response, status, errorEnvelope(externalRpcId, code, message));
        request.resume();
      } finally {
        if (admitted) active -= 1;
        clearTimeout(timeout);
        controller.signal.removeEventListener('abort', stop);
        response.off('close', disconnected);
      }
    }
  } catch {
    await dispose().catch(() => {});
    throw new Error('Bridge startup failed; secure discovery or DSH connection is unavailable');
  }
}

/** Cordis plugin entry: register cleanup synchronously, including async startup races. */
export function apply(ctx, config = {}) {
  if (!keysAre(config, [], ['profileLabel'])) throw new Error('Invalid bridge configuration');
  validateLabel(config.profileLabel);
  const lifecycle = new AbortController();
  let ready;
  ctx.effect(() => {
    ready = startBridge({ connection: ctx.connection, profileLabel: config.profileLabel, signal: lifecycle.signal });
    ready.catch(() => {});
    return async () => {
      lifecycle.abort();
      const bridge = await ready.catch(() => undefined);
      await bridge?.dispose();
    };
  }, 'codex-usage-overlay-bridge: loopback companion');
  return ready.then(() => undefined);
}
