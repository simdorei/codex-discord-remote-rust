# Standalone browser access host candidate (Windows)

The candidate Rust runtime embeds the browser host and registers its internal
`--official-browser-host` MCP entrypoint when the managed official manifest
declares a browser surface. It runs the installed official CUA server behind a
transparent stdio wrapper. It uses the Node executable already included in that
official installation; no npm dependencies or separately copied scripts are needed.
Production deployment remains separate from building and testing this candidate.

The compiled-candidate real-turn test on 2026-11-17 succeeded with the ChatGPT GUI
absent: official Chrome extension discovery, navigation to a local fixture,
text entry, button click, visible result verification, and disposable-tab cleanup.
This does not establish full desktop-app feature parity or all-site compatibility.

## Files and trust boundary

- `../../crates/cdr-runtime/src/local_computer/browser_host/authority.mjs`: owns a fresh Ed25519 key, executor namespace and Windows named
  pipe. Only an outstanding official `tools/call` for `js`, with caller-supplied
  `x-codex-turn-metadata`, admits that exact session/turn. Metadata in tool
  arguments cannot grant access. Completion, cancellation, shutdown and expiry
  retire admission. Another launch's executor is rejected.
- `../../crates/cdr-runtime/src/local_computer/browser_host/launcher.mjs`: validates the selected official launcher and guard, starts the
  host before its one owned CUA child, and forwards MCP messages byte-for-byte.
  It does not generate elicitation decisions, change site permissions, suppress
  safety review, remove the browser guard, or connect via a different provider.
  Inherited GUI host context is discarded; the child receives this host's fresh
  complete tuple. Private keys stay in memory.
- `authority.test.mjs`: denial, cancellation, namespace isolation, signature,
  framing and approval-forwarding regressions, including real Windows pipes.
- `probe.py`: isolated real model-turn acceptance runner, contained in its own
  kill-on-close Windows job. Any client approval request waits for an explicit
  matching reply file. It never automatically accepts requests. An optional
  loopback-only page records the result of its actual button handler.
- `local_computer/browser_host.rs` and `browser_host/cache.rs` in the runtime
  crate: select the compiled runtime as MCP launcher, publish embedded helpers
  atomically under a content digest in `CODEX_HOME/cdr-browser-host`, verify their
  exact bytes on reuse, and bind launch to the selected manifest digest. Changed
  existing cache files are rejected rather than overwritten. The helper mode
  runs before the bot's Tokio runtime, Discord configuration or DB startup.

The host answers **whether the calling local MCP session belongs to its current
execution**, not whether a website operation should be approved. The official
service continues to enforce the latter. The host is our implementation; it is
not an OpenAI-issued executor, and it does not copy a desktop chat's identity.

## Run focused checks

Use the Node executable declared by the chosen official `.mcp.json`:

```powershell
& $bundledNode --test experiments/browser-host/authority.test.mjs
```

The managed Rust registration uses the same installed-bundle selection and
official guard as before. Its native-only manifest and disabled-CUA paths retain
their original launch behavior. An invalid host cache follows the existing
optional-CUA failure path and does not prevent ordinary Discord/JS operation.
The internal launcher starts the original `--import <official guard> <official
CUA entrypoint>` unchanged. It never rewrites the installed manifest or user config.
Use the compiled candidate in the isolated real-turn probe:

```powershell
python -X utf8 experiments/browser-host/probe.py `
  --runtime $candidateRuntime `
  --manifest $officialManifest --output $newEvidenceDirectory `
  --prompt $reviewedTestPrompt --local-page
```

The prompt may contain `{TEST_URL}`, which is replaced with the fixture's
loopback URL. Browser tests affect the real desktop and must preserve existing
tabs, shared tab limits and current user work. The probe does not close the GPT
app or start/stop the production Discord bot or external MCP agent.

## Current limits before production deployment

- The Rust startup path now includes the host, and the helpers travel inside
  `cdr-runtime.exe`. The existing Windows release payload already includes that
  executable. A deliberate production rollout and release approval remain.
- Validated against CLI `0.167.0-alpha.4.1`, official CUA bundle
  `26.1106.51428`, and Chrome extension `1.26.1102.11740` on this Windows PC.
  The local access protocol is version-specific, not a documented stable public
  host-registration API. Revalidate with changed bundles.
- Only active MCP calls are admitted. Calls that outlive a returned tool result
  are not newly admitted. The official browser service also caches checks within
  a turn; this wrapper is not a replacement for its cancellation/turn cleanup.
- Stalled admissions expire after ten minutes. Frames, simultaneous calls,
  connections and replay tracking are bounded. These limits need workload review
  before general deployment.
- Windows uses the named pipe's default process-token DACL plus an unguessable
  per-launch endpoint and executor. Explicit peer-token validation, cross-user
  adversarial tests, abrupt parent-kill behavior outside the contained probe,
  and installer/update compatibility remain production review items.
- Existing site/operation approval forwarding was exercised; no new real denial
  was deliberately requested from the live approval reviewer. Decline/cancel
  byte preservation and host-denial behavior were tested locally.

Evidence and the operational-test incident are documented in
[`../../docs/standalone-browser-host-review-20261117.md`](../../docs/standalone-browser-host-review-20261117.md).
