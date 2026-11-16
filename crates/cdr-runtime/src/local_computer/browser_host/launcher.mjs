import { spawn } from 'node:child_process';
import { readFile, realpath, stat } from 'node:fs/promises';
import { openSync, writeSync, closeSync } from 'node:fs';
import { createHash, randomUUID } from 'node:crypto';
import { parseArgs } from 'node:util';
import { isAbsolute, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Transform } from 'node:stream';
import { BrowserAuthority, listenAuthority } from './authority.mjs';

// Keep every MCP message byte-for-byte, including elicitation requests,
// decisions, metadata, cancellation and tool errors. Inspect only ownership.
export class McpTap extends Transform {
  #pending = Buffer.alloc(0);
  constructor(observe) { super(); this.observe = observe; }
  _transform(chunk, encoding, callback) {
    try {
      this.#pending = Buffer.concat([this.#pending, chunk]);
      for (;;) {
        const end = this.#pending.indexOf(10);
        if (end < 0) break;
        if (end > 32 * 1024 * 1024) throw new Error('MCP frame exceeds experiment limit');
        const line = this.#pending.subarray(0, end + 1);
        const text = line.toString('utf8').trim();
        if (text) this.observe(JSON.parse(text));
        this.push(line);
        this.#pending = this.#pending.subarray(end + 1);
      }
      if (this.#pending.length > 32 * 1024 * 1024) throw new Error('MCP frame exceeds experiment limit');
      callback();
    } catch (error) { callback(error); }
  }
  _flush(callback) {
    callback(this.#pending.length ? new Error('truncated MCP frame') : undefined);
  }
}

export async function loadLaunch(manifestPath, expectedManifestHash) {
  if (!isAbsolute(manifestPath)) throw new Error('manifest path must be absolute');
  if ((await stat(manifestPath)).size > 1024 * 1024) throw new Error('manifest too large');
  const bytes = await readFile(manifestPath);
  if (expectedManifestHash !== undefined &&
      (!/^[a-f0-9]{64}$/.test(expectedManifestHash) || createHash('sha256').update(bytes).digest('hex') !== expectedManifestHash)) {
    throw new Error('official CUA manifest changed after selection');
  }
  const manifest = JSON.parse(bytes.toString('utf8'));
  const config = manifest.mcpServers?.cua_repl;
  if (!config || !isAbsolute(config.command) || !Array.isArray(config.args) || config.args.length !== 3) {
    throw new Error('unsupported official CUA manifest');
  }
  const [flag, guardUrl, entry] = config.args;
  const bin = dirname(config.command);
  const expectedEntry = join(bin, 'node_modules/@oai/cua-repl/bin/cua-repl.mjs');
  const guard = join(dirname(manifestPath), 'require-browser-access.mjs');
  if (flag !== '--import' || !guardUrl.startsWith('file:') ||
      (await realpath(fileURLToPath(guardUrl))).toLowerCase() !== (await realpath(guard)).toLowerCase() ||
      (await realpath(entry)).toLowerCase() !== (await realpath(expectedEntry)).toLowerCase() ||
      (await realpath(config.command)).toLowerCase() !== (await realpath(process.execPath)).toLowerCase()) {
    throw new Error('launcher/guard must match the selected official bundle');
  }
  const env = { ...process.env, ...config.env };
  for (const name of Object.keys(env)) {
    if (/^(BROWSER_USE_ACCESS_|CODEX_APP_TOOLS_PIPE_PATH$|CODEX_TASK_WORKSPACE_VERIFYING_IDENTITY$|CODEX_SESSION_ID$|CODEX_THREAD_ID$|CODEX_INTERNAL_ORIGINATOR_OVERRIDE$|SKY_CUA_NATIVE_PIPE_DIRECTORY$|SKY_CUA_NATIVE_PIPE$|OAI_SKY_WINDOWS_BIN$)/i.test(name)) delete env[name];
  }
  // Match Rust Remote's existing, explicit native-helper selection. Browser
  // guard, backends, trusted services and approval policy are not modified.
  env.SKY_CUA_NATIVE_PIPE = '0';
  env.SKY_CUA_NATIVE_PIPE_DIRECTORY = '';
  const arch = process.arch === 'arm64' ? 'arm64' : 'x64';
  env.OAI_SKY_WINDOWS_BIN = join(bin, `node_modules/@oai/sky/bin/windows/swift/${arch}/codex-computer-use-swift.exe`);
  for (const path of [config.command, env.CODEX_CLI_PATH, env.CUA_REPL_NODE_REPL_PATH, env.OAI_SKY_WINDOWS_BIN]) {
    if (typeof path !== 'string' || !isAbsolute(path) || !(await stat(path)).isFile()) throw new Error('incomplete CUA bundle');
  }
  if (!config.env?.CUA_REPL_ENABLED_SURFACES?.split(',').map(value => value.trim()).includes('browser')) throw new Error('browser surface missing');
  return { command: config.command, args: config.args, env };
}

async function main() {
  const { values } = parseArgs({ options: {
    manifest: { type: 'string' }, 'expected-manifest-sha256': { type: 'string' },
    'audit-directory': { type: 'string' },
  } });
  const manifest = values.manifest, auditDirectory = values['audit-directory'];
  if (process.platform !== 'win32' || !manifest || !isAbsolute(manifest) ||
      (auditDirectory && !isAbsolute(auditDirectory))) {
    throw new Error('An absolute official manifest and optional absolute audit directory are required on Windows');
  }
  const launch = await loadLaunch(manifest, values['expected-manifest-sha256']);
  const auditFd = auditDirectory ? openSync(join(auditDirectory, `host-${process.pid}-${randomUUID()}.jsonl`), 'wx') : undefined;
  let auditClosed = false;
  const audit = event => {
    if (auditFd !== undefined && !auditClosed) writeSync(auditFd, JSON.stringify({ at: new Date().toISOString(), ...event }) + '\n');
  };
  const authority = new BrowserAuthority();
  const host = await listenAuthority(authority, { onDecision: allowed => {
    audit({ event: 'access', allowed });
    console.error(`cdr_browser_host access=${allowed ? 'owned_active_call' : 'denied'}`);
  } });
  const child = spawn(launch.command, launch.args, {
    env: { ...launch.env, ...host.environment }, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true,
  });
  let closing;
  const shutdown = () => closing ??= (async () => {
    authority.close();
    process.stdin.unpipe();
    child.stdin.end();
    await host.close();
    // Retain the owned child handle while terminating its tree. Never target
    // an existing app, bot, native-messaging host or external MCP process.
    if (child.exitCode === null && child.signalCode === null && child.pid) {
      await new Promise(resolveKill => {
        const killer = spawn('taskkill.exe', ['/PID', String(child.pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore' });
        killer.once('error', resolveKill);
        killer.once('close', resolveKill);
      });
    }
    process.stdin.destroy();
    audit({ event: 'closed' });
    auditClosed = true;
    if (auditFd !== undefined) closeSync(auditFd);
  })();
  const fail = error => {
    console.error(`cdr_browser_host stopped: ${error.message}`);
    process.exitCode = 1;
    void shutdown();
  };
  const pendingApprovals = new Set();
  const parentTap = new McpTap(message => {
    authority.observeParent(message);
    if (message.method === 'tools/call') audit({ event: 'tool', name: message.params?.name });
    if (!('method' in message) && pendingApprovals.delete(JSON.stringify(message.id))) {
      const action = message.result?.action;
      audit({ event: 'approval_response_forwarded', action: ['accept', 'decline', 'cancel'].includes(action) ? action : 'other' });
    }
  });
  const childTap = new McpTap(message => {
    authority.observeChild(message);
    if (message.method === 'elicitation/create') {
      if (pendingApprovals.size >= 256) throw new Error('too many outstanding approvals');
      pendingApprovals.add(JSON.stringify(message.id));
      audit({ event: 'approval_forwarded' });
    }
  });
  for (const stream of [parentTap, childTap, child.stdin, child.stdout, process.stdout]) stream.on('error', fail);
  child.once('error', fail);
  child.stderr.pipe(process.stderr, { end: false });
  process.stdin.pipe(parentTap).pipe(child.stdin);
  child.stdout.pipe(childTap).pipe(process.stdout, { end: false });
  process.stdin.once('end', () => void shutdown());
  for (const signal of ['SIGINT', 'SIGTERM', 'SIGHUP']) process.once(signal, () => void shutdown());
  child.once('exit', code => { if (!closing) process.exitCode = code ?? 1; void shutdown(); });
  audit({ event: 'ready', hostPid: process.pid, childPid: child.pid });
  console.error('cdr_browser_host ready scope=one_owned_cua_child approval_passthrough=unchanged');
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(`cdr_browser_host startup failed: ${error.message}`); process.exitCode = 1; });
}
