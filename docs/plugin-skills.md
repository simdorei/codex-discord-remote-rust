## Bundled Skills And Attribution

The Codex Discord Remote plugin packages these skills. In Codex, the fully qualified
skill names use the `$codex-discord-remote:<skill-name>` form. In Discord,
some skills also have slash or `!` command wrappers.

| Skill | Purpose | Discord entrypoint |
| --- | --- | --- |
| `ask-chatgpt-pro` | Reuse one normal ChatGPT Pro conversation per Codex thread, bind it to the originating local project through OAuth-protected MCP, and optionally launch session-owned isolated Chrome or operate blank Notepad with short-lived screenshot tokens. Chrome pixels remain unavailable. A missing saved conversation is replaced. | `!pro <question>` invokes the skill from a mapped Discord thread. |
| `discord-remote` | Operational runbook for this local Discord bridge: bot status, watchdog restarts, archive-lock recovery, mirror routing checks, log triage, and local deployment checks. | No direct skill wrapper; use the normal remote commands such as `!status`, `!doctor`, `!mirror check`, and `!bridge sync`. |
| `discord-remote-qa` | Focused QA checklist for mirror mapping, context refresh, session mirror cursor priming, steering suppression, archive lock retry, and deployment readiness. | No direct skill wrapper; use it from Codex when validating remote changes. |
| `intent-driven-qa` | Converts intended behavior into a traceable automated test contract, selects the cheapest trustworthy test layer, enforces honest RED-to-GREEN evidence, and audits whether tests catch realistic defects. | Send a normal prompt in a mapped Discord thread and invoke `$codex-discord-remote:intent-driven-qa`; `!qa` remains reserved for bridge button QA. |
| `archive-used` | Bulk archive workflow for Codex threads whose `used` value in bridge list output is at or above a user-provided `<threshold>`, targeting the UUID printed in each selected list row. | `!archive-used <threshold>` invokes the skill from Discord; Codex then uses local bridge list/archive commands or the equivalent `!list` and `!archive <uuid-from-list>`. |
| `deep-interview` | Clarification-first requirements workflow. It confirms the work structure, asks one question at a time, scores ambiguity, preserves the user's language, tracks scope/entities/constraints, and stops at a pending-approval ticket before implementation. | `/interview <request>` or `!interview <request>`. |

### QA And Review Roles

`intent-driven-qa` is a general software QA methodology packaged with this plugin:
it designs defect-detecting tests from intended behavior. `discord-remote-qa`
adds checks specific to the Discord bridge.

The companion [Repo Review Gate](../.agents/skills/repo-review-gate/SKILL.md)
controls review order: inspect existing execution evidence, run missing or
invalidated checks, then review uncovered paths. Its source is maintained under
`.agents/skills/repo-review-gate` as a project skill, outside the plugin's bundled
skill directory. It can also be installed personally for use across repositories.

Select the workflow needed by the task. When combining them, share valid execution
evidence and add only the missing coverage; these skills do not require repeating
the same checks in separate QA and review stages.

### Requesting Intent-Driven QA

In a mapped Discord thread or a Codex task, give the behavior and evidence you care about; let the skill choose how many tests belong at each layer. Use the fully qualified plugin skill name from Discord:

```text
$codex-discord-remote:intent-driven-qa

실제로 원하는 결과:
반드시 지켜야 할 규칙/불변조건:
실패하면 특히 위험한 상황:
범위와 비범위:
원하는 모드: 계약만 / 테스트만 / 테스트 후 구현 / 버그 회귀 고정 / 기존 테스트 감사
수정 가능한 범위:
필요한 증거: RED, GREEN, 관련 회귀 테스트, 잔여 위험
```

For example:

```text
$codex-discord-remote:intent-driven-qa 초대 링크는 한 번만 사용할 수 있고 24시간 뒤 만료되며 다른 tenant 사용자는 쓸 수 없어. 먼저 실패 테스트가 의도한 이유로 RED인지 보여준 뒤 최소 구현하고, 같은 테스트와 관련 suite의 GREEN 증거를 줘. 테스트 계약을 바꿔야 하면 구현 전에 이유를 알려줘.
```

For an audit without product changes:

```text
$codex-discord-remote:intent-driven-qa 주문 모듈 테스트가 가격 계산, 권한, 중복 결제 회귀를 실제로 잡는지 감사해줘. 커버리지 수치보다 oracle 독립성, mock 경계, assertion 강도, 경계값을 보고 제품 코드는 수정하지 마.
```

Source attribution is included for transparency and license compliance. These
upstream authors are not listed as this repository's contributors unless they
contributed directly here; the notes below identify inspiration, adaptation, or
vendored source material only.

- `deep-interview` is adapted from the Gajae Code deep-interview skill:
  https://github.com/Yeachan-Heo/gajae-code/tree/main/packages/coding-agent/src/defaults/gjc/skills/deep-interview
- Gajae Code is MIT licensed. The packaged notice is at
  `plugins/codex-discord-remote/skills/deep-interview/NOTICE.md`.
- Repository-level third-party notices are collected in `NOTICE.md`.

Useful plugin-backed scripts can also be run directly from the repository:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\plugins\codex-discord-remote\scripts\status.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\plugins\codex-discord-remote\scripts\restart.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\plugins\codex-discord-remote\scripts\qa-smoke.ps1 -SkipUnitTests
```

This repository uses the Rust runtime only. Status and restart entrypoints
dispatch to the Rust-specific scripts; Python runtime rollback has been removed.
Restart and deployment tools check runtime ownership and readiness before
replacement. Inspect each script's parameters and use its dry-run facilities before
operating a live installation. These tools are not invoked by cloning this repository.
