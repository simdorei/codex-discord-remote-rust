import { generateKeyPairSync, randomBytes, sign } from 'node:crypto';
import { createServer } from 'node:net';

const MAX_FRAME = 16 * 1024;
const MAX_CALLS = 64;
const LEASE_MS = 10 * 60 * 1000;
const identifier = value => typeof value === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value);
const requestId = value => typeof value === 'string' || Number.isSafeInteger(value);
const key = value => JSON.stringify(value);

// Embedded in cdr-runtime. This host owns one MCP child. Only metadata supplied on its private parent
// stdio channel can admit a call. Model code/tool arguments cannot grant access.
// This is a local ownership decision, NOT a site/operation approval: those stay
// in the unmodified official service and travel over MCP without interception.
export class BrowserAuthority {
  #privateKey;
  #calls = new Map();
  #closed = false;
  #now;

  constructor({ now = Date.now } = {}) {
    const { privateKey, publicKey } = generateKeyPairSync('ed25519');
    this.#privateKey = privateKey;
    this.verifyingIdentity = publicKey.export({ type: 'spki', format: 'der' }).toString('base64');
    this.executorId = `cdr-local-${randomBytes(32).toString('hex')}`;
    this.#now = now;
  }

  observeParent(message) {
    if (message.method === 'notifications/cancelled') {
      this.#calls.delete(key(message.params?.requestId));
      return;
    }
    if (message.method !== 'tools/call' || !requestId(message.id)) return;
    // Reusing an outstanding ID makes response ownership ambiguous.
    if (this.#calls.has(key(message.id))) throw new Error('duplicate outstanding MCP request ID');
    if (message.params?.name !== 'js') return;
    let metadata = message.params?._meta?.['x-codex-turn-metadata'];
    if (typeof metadata === 'string') {
      try { metadata = JSON.parse(metadata); } catch { return; }
    }
    if (!metadata || !identifier(metadata.session_id) || !identifier(metadata.turn_id)) return;
    if (this.#closed || this.#calls.size >= MAX_CALLS) throw new Error('browser host admission unavailable');
    this.#calls.set(key(message.id), {
      session: metadata.session_id, turn: metadata.turn_id,
      deadline: this.#now() + LEASE_MS, nonces: new Set(),
    });
  }

  observeChild(message) {
    // Server-to-client approval requests have their own ID namespace. They
    // must never retire a client call that happens to use the same numeric ID.
    if (!('method' in message) && requestId(message.id)) this.#calls.delete(key(message.id));
  }

  close() {
    this.#closed = true;
    this.#calls.clear();
  }

  respond(message) {
    if (!message || message.jsonrpc !== '2.0' || !requestId(message.id)) return null;
    if (message.method !== 'browser/access') {
      return { jsonrpc: '2.0', id: message.id, error: { code: -32601, message: 'Unsupported host method' } };
    }
    const p = message.params;
    if (!p || Array.isArray(p) || Object.keys(p).length !== 4 ||
        typeof p.executorId !== 'string' || !identifier(p.sessionId) || !identifier(p.turnId) ||
        typeof p.nonce !== 'string' || !/^[a-f0-9]{64}$/.test(p.nonce)) {
      return { jsonrpc: '2.0', id: message.id, error: { code: -32602, message: 'Invalid access request' } };
    }
    let allowed = false;
    if (!this.#closed && p.executorId === this.executorId) {
      const matches = [...this.#calls.values()].filter(call =>
        call.session === p.sessionId && call.turn === p.turnId && call.deadline > this.#now());
      if (matches.length && !matches.some(call => call.nonces.has(p.nonce)) &&
          matches.every(call => call.nonces.size < 256)) {
        for (const call of matches) call.nonces.add(p.nonce);
        allowed = true;
      }
    }
    const payload = JSON.stringify({ domain: 'codex.browserAccess.v1', request: p, allowed });
    return { jsonrpc: '2.0', id: message.id, result: {
      payload, signature: sign(null, Buffer.from(payload), this.#privateKey).toString('base64'),
    } };
  }
}

export function encodeFrame(message) {
  const body = Buffer.from(JSON.stringify(message));
  if (body.length > MAX_FRAME) throw new Error('access frame too large');
  const header = Buffer.alloc(4);
  header.writeUInt32LE(body.length);
  return Buffer.concat([header, body]);
}

export async function listenAuthority(authority, { onDecision = () => {} } = {}) {
  if (process.platform !== 'win32') throw new Error('This experiment requires Windows named pipes');
  // A per-launch capability name. No discovery file, global endpoint, or
  // GUI-owned key is used. Node's default Windows pipe DACL is retained.
  const pipePath = `\\\\.\\pipe\\cdr-browser-access-${randomBytes(32).toString('hex')}`;
  const sockets = new Set();
  const server = createServer(socket => {
    if (sockets.size >= 16) { socket.destroy(); return; }
    sockets.add(socket);
    let data = Buffer.alloc(0);
    socket.setTimeout(5000, () => socket.destroy());
    socket.on('error', () => {});
    socket.once('close', () => sockets.delete(socket));
    socket.on('data', chunk => {
      // One bounded request per connection, like the official access client.
      if (data.length + chunk.length > MAX_FRAME + 4) { socket.destroy(); return; }
      data = Buffer.concat([data, chunk]);
      if (data.length < 4) return;
      const size = data.readUInt32LE();
      if (!size || size > MAX_FRAME) { socket.destroy(); return; }
      if (data.length < size + 4) return;
      if (data.length !== size + 4) { socket.destroy(); return; }
      try {
        const reply = authority.respond(JSON.parse(data.subarray(4).toString('utf8')));
        if (!reply) { socket.destroy(); return; }
        onDecision(reply.result ? JSON.parse(reply.result.payload).allowed : false);
        socket.end(encodeFrame(reply));
      } catch { socket.destroy(); }
    });
  });
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(pipePath, resolve);
  });
  return {
    environment: {
      BROWSER_USE_ACCESS_PIPE_PATH: pipePath,
      BROWSER_USE_ACCESS_EXECUTOR_ID: authority.executorId,
      BROWSER_USE_ACCESS_VERIFYING_IDENTITY: authority.verifyingIdentity,
    },
    async close() {
      authority.close();
      for (const socket of sockets) socket.destroy();
      await new Promise(resolve => server.close(resolve));
    },
  };
}
