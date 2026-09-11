import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
import test from 'node:test';

const fixture = fileURLToPath(new URL('./connector_fixture.mjs', import.meta.url));
const source = new URL('../../../plugins/codex-discord-remote/skills/ask-chatgpt-pro/scripts/pro_connector_control.mjs', import.meta.url).href;
export function run(config = {}) {
  const child = spawnSync(process.execPath, [fixture, JSON.stringify(config), source], { encoding: 'utf8', timeout: 20000, maxBuffer: 1024 * 1024 });
  assert.equal(child.error, undefined);
  assert.equal(child.status, 0, child.stderr);
  return JSON.parse(child.stdout);
}
export function check(result, fields, calls, includes = [], excludes = []) {
  for (const [key, value] of Object.entries(fields)) assert.deepEqual(result.evidence[key], value, key);
  if (calls !== undefined) assert.deepEqual(result.calls, calls);
  for (const call of includes) assert.ok(result.calls.includes(call), `missing ${call}`);
  for (const call of excludes) assert.ok(!result.calls.includes(call), `forbidden ${call}`);
}
export function contract(name, config, fields, calls, includes, excludes) {
  test(name, () => check(run(config), fields, calls, includes, excludes));
}
export function failed(name, config, stage, calls, excludes = []) {
  contract(name, config, { status: 'failed', failed_stage: stage }, calls, [], excludes);
}
export { assert, test };
