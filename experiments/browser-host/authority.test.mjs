import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createPublicKey, verify, randomBytes } from 'node:crypto';
import { createConnection } from 'node:net';
import { once } from 'node:events';
import { BrowserAuthority, encodeFrame, listenAuthority } from '../../crates/cdr-runtime/src/local_computer/browser_host/authority.mjs';
import { McpTap, loadLaunch } from '../../crates/cdr-runtime/src/local_computer/browser_host/launcher.mjs';
import { mkdtemp, writeFile, rm, rmdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const metadata = { session_id: 'owned-session', turn_id: 'owned-turn' };

test('a changed manifest is rejected before parsing or launching its commands', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'cdr-manifest-test-'));
  const path = join(directory, '.mcp.json');
  try {
    await writeFile(path, '{}');
    await assert.rejects(loadLaunch(path, '0'.repeat(64)), /changed after selection/);
  } finally { await rm(path, { force: true }); await rmdir(directory); }
});
function call(id = 1, meta = metadata) {
  return { jsonrpc: '2.0', id, method: 'tools/call', params: {
    name: 'js', arguments: { code: 'await cua.getState()' },
    _meta: { 'x-codex-turn-metadata': JSON.stringify(meta) },
  } };
}
function request(host, overrides = {}) {
  return { jsonrpc: '2.0', id: 1, method: 'browser/access', params: {
    executorId: host.executorId, sessionId: metadata.session_id, turnId: metadata.turn_id,
    nonce: randomBytes(32).toString('hex'), ...overrides,
  } };
}
function result(host, req = request(host)) {
  const reply = host.respond(req);
  const { payload, signature } = reply.result;
  const publicKey = createPublicKey({ key: Buffer.from(host.verifyingIdentity, 'base64'), type: 'spki', format: 'der' });
  assert.equal(verify(null, Buffer.from(payload), publicKey, Buffer.from(signature, 'base64')), true);
  const parsed = JSON.parse(payload);
  assert.equal(payload, JSON.stringify({ domain: 'codex.browserAccess.v1', request: req.params, allowed: parsed.allowed }));
  return parsed.allowed;
}

test('an actual MCP call must be active; completion retires it', () => {
  const host = new BrowserAuthority();
  assert.equal(result(host), false);
  host.observeParent(call());
  assert.equal(result(host), true);
  host.observeChild({ jsonrpc: '2.0', id: 1, result: { content: [] } });
  assert.equal(result(host), false);
});

test('tool arguments and missing or malformed metadata grant nothing', () => {
  for (const meta of [undefined, [], {}, { ...metadata, session_id: '' }, 'broken']) {
    const host = new BrowserAuthority();
    const message = call();
    message.params._meta = { 'x-codex-turn-metadata': meta };
    message.params.arguments._meta = { 'x-codex-turn-metadata': metadata };
    host.observeParent(message);
    assert.equal(result(host), false);
  }
});

test('another executor, session, or turn remains denied', () => {
  const host = new BrowserAuthority();
  host.observeParent(call());
  for (const overrides of [{ executorId: 'other' }, { sessionId: 'other' }, { turnId: 'other' }]) {
    assert.equal(result(host, request(host, overrides)), false);
  }
});

test('approval requests with overlapping IDs do not retire active calls', () => {
  const host = new BrowserAuthority();
  host.observeParent(call());
  host.observeChild({ jsonrpc: '2.0', id: 1, method: 'elicitation/create', params: { message: 'Allow this site?' } });
  host.observeParent({ jsonrpc: '2.0', id: 1, result: { action: 'decline' } });
  assert.equal(result(host), true);
});

test('cancellation and shutdown revoke outstanding admissions', () => {
  const host = new BrowserAuthority();
  host.observeParent(call());
  host.observeParent({ method: 'notifications/cancelled', params: { requestId: 1 } });
  assert.equal(result(host), false);
  host.observeParent(call(2));
  host.close();
  assert.equal(result(host), false);
  assert.throws(() => host.observeParent(call(3)), /admission unavailable/);
});

test('stalled calls expire and duplicate outstanding IDs fail closed', () => {
  let time = 0;
  const host = new BrowserAuthority({ now: () => time });
  host.observeParent(call());
  assert.throws(() => host.observeParent(call()), /duplicate/);
  time = 600001;
  assert.equal(result(host), false);
});

test('replay and malformed requests cannot produce an allow', () => {
  const host = new BrowserAuthority();
  host.observeParent(call());
  const req = request(host);
  assert.equal(result(host, req), true);
  assert.equal(result(host, req), false);
  for (const overrides of [{ nonce: '' }, { unexpected: true }, { turnId: 42 }]) {
    assert.equal(host.respond(request(host, overrides)).error.code, -32602);
  }
  assert.equal(host.respond({ ...req, method: 'tools/call' }).error.code, -32601);
});

test('each host has a fresh identity and does not accept another launch', () => {
  const a = new BrowserAuthority();
  const b = new BrowserAuthority();
  a.observeParent(call()); b.observeParent(call());
  assert.notEqual(a.verifyingIdentity, b.verifyingIdentity);
  assert.notEqual(a.executorId, b.executorId);
  assert.equal(result(b, request(a)), false);
});

test('MCP tap preserves UTF-8, approval metadata, declines, cancellation byte-for-byte', async () => {
  const messages = [
    call(),
    { jsonrpc: '2.0', id: 9, method: 'elicitation/create', params: {
      message: '사이트 접근 승인', _meta: { codex_strict_auto_review: true, codex_requires_user_input: true },
    } },
    { jsonrpc: '2.0', id: 9, result: { action: 'decline', _meta: { message: '거절' } } },
    { jsonrpc: '2.0', method: 'notifications/cancelled', params: { requestId: 1 } },
  ];
  const input = Buffer.from(messages.map(m => JSON.stringify(m) + '\r\n').join(''));
  const observed = [], output = [];
  const tap = new McpTap(m => observed.push(m));
  tap.on('data', data => output.push(data));
  const done = once(tap, 'end');
  for (let i = 0; i < input.length; i += 7) tap.write(input.subarray(i, i + 7));
  tap.end(); await done;
  assert.deepEqual(Buffer.concat(output), input);
  assert.deepEqual(observed, messages);
});

test('truncated MCP input fails instead of silently changing a request', async () => {
  const tap = new McpTap(() => {});
  const error = once(tap, 'error');
  tap.resume(); tap.end('{"id":1');
  assert.match((await error)[0].message, /truncated/);
});

test('Windows named pipe uses framed signed responses and closes on shutdown', { skip: process.platform !== 'win32' }, async () => {
  const authority = new BrowserAuthority();
  authority.observeParent(call());
  const host = await listenAuthority(authority);
  const client = createConnection(host.environment.BROWSER_USE_ACCESS_PIPE_PATH);
  client.on('error', () => {});
  try {
    await once(client, 'connect');
    const chunks = [];
    client.on('data', chunk => chunks.push(chunk));
    const ended = once(client, 'end');
    const frame = encodeFrame(request(authority));
    client.write(frame.subarray(0, 2)); client.write(frame.subarray(2));
    await ended;
    const data = Buffer.concat(chunks);
    assert.equal(data.readUInt32LE(), data.length - 4);
    assert.equal(JSON.parse(JSON.parse(data.subarray(4)).result.payload).allowed, true);
  } finally { client.destroy(); await host.close(); }
  assert.equal(result(authority), false);
});

test('Windows pipe rejects oversized and malformed requests without authorizing', { skip: process.platform !== 'win32' }, async () => {
  const authority = new BrowserAuthority();
  authority.observeParent(call());
  const decisions = [];
  const host = await listenAuthority(authority, { onDecision: value => decisions.push(value) });
  try {
    const oversized = Buffer.alloc(4); oversized.writeUInt32LE(1024 * 1024);
    const malformed = Buffer.from([1, 0, 0, 0, 123]);
    for (const frame of [oversized, malformed]) {
      const client = createConnection(host.environment.BROWSER_USE_ACCESS_PIPE_PATH);
      await once(client, 'connect');
      const ended = once(client, 'close');
      client.on('data', () => assert.fail('invalid request received a response'));
      client.end(frame);
      await ended;
    }
    assert.deepEqual(decisions, []);
  } finally { await host.close(); }
});
