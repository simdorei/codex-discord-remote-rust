# Frozen legacy contracts (Python-free tests)

Source snapshot: `3b359b470fee412bf010825c0e64a6139ab03b3c` in the Rust publication
repository, before this migration. These fixtures must not be regenerated from the
implementation being tested.

| Legacy source | SHA-256 of the checked-out source | Replacement |
| --- | --- | --- |
| `codex_discord_prefix_skill_prompts.py` | `11A4A51BE882AA120AFD0149F238D92930DDD22EDD8E38BC994AB14BA6246029` | `deep_interview_header.txt`, verbatim header including final newline |
| `codex_discord_store_schema.py` | `57F67A8A14E0FD3EB129B8ABE79A382A08321EEE988577CFCB55D291DFF07B7B` | Existing independently captured `discord_mirror_schema_v2.sql` |
| `codex_discord_store_processed_messages.py` | `7EE47954C89E1B06DD671648B8CA4B1E60EB13AA545626E69FFB4646BD822B95` | Frozen INSERT OR IGNORE / INSERT OR REPLACE and permanent replay-row contracts |
| `remote_mcp_server/simdorei_mcp/oauth_store.py` | `07DD92A5B60D061671C758A8E553B4C56DB72BF54258691A8B136E3C246450FA` | `legacy_oauth_store.sql`, verbatim DDL and synthetic legacy-format token rows |

The OAuth fixture hashes refer only to the explicit synthetic test strings
`python-access` and `python-refresh`; they are not credentials. Tests inspect Rust
output using the frozen legacy lookup/JSON/nullability contract, then reopen the
old rows using Rust. No interpreter or MCP Python dependencies are executed.

The legacy-runtime rollback tests are retired with the user-approved removal of
that runtime. Their supported **data compatibility and replay protection** are
retained in `legacy_store_contract.rs`, including fresh/empty v2 schemas, exact
columns, claim-only and marked rows, Rust extension preservation, and backup/reopen.
These are not represented as proof that a removed Python runtime still executes.

The subprocess scenarios under `crates/cdr-runtime/tests/fixtures/app_server/`
were translated from the corresponding pre-migration test helpers. They run in
the existing `cdr-offline-soak` executable with `--app-server-fixture`, use real
stdin/stdout JSON-RPC, preserve gates, errors and persisted SQLite/rollout effects,
and never start the real Codex app. Existing product assertions are unchanged.
For a clean `cargo test --lib`-only run, first build `cdr-offline-soak`; ordinary
integration/workspace test builds already build that binary.
