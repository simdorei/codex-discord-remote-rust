---
name: repo-review-gate
description: Review workflow for software repositories. Use when Codex is asked to review code, hunt regressions, harden externally reachable flows, or reduce repeated exploratory re-review. Discover and run repo-local deterministic review gates first, then do only a short residual critical review for paths not covered by those gates.
---

# Repo Review Gate

1. Confirm the repository root and read the root `AGENTS.md` if present.
2. Before manual review, look for repo-local deterministic gates in this order:
   - named review harness scripts such as `scripts/check_*review*`, `scripts/check_*harness*`, `scripts/check_*regression*`
   - existing CI-equivalent local checks already used by the repo
   - focused compile/import/route/test commands for the changed surface
3. Inspect existing local/CI results before running the narrowest gate that matches the review target. Reuse a retrievable execution when its covered scope, tested source (including relevant dirty changes), configuration, dependencies, and environment still match. Run only missing or invalidated checks; a broader valid run can cover included checks. Share the same evidence across reviewers and skills instead of executing it again for each. Prefer existing repo scripts over inventing a new checklist in the turn.
4. If no deterministic gate exists, assemble a minimal one from local repo conventions:
   - syntax/import check on touched modules
   - route or registration sanity for web apps
   - one abuse-path regression for public/auth/session changes
5. If a gate fails, distinguish a product defect from a fixture/runner failure or missing evidence, then fix or report the failed step. Recover an inaccessible log or artifact identity before deciding to rerun tests. Keep unaffected results; do not restart all checks for an evidence-only repair. Missing evidence remains unverified. Do not continue into broad exploratory review while the deterministic gate is red.
6. If the gate passes, do only a short residual critical review for uncovered paths:
   - new externally reachable routes or screens
   - trusted-boundary fallbacks such as host, proxy, redirect, callback, or base-url derivation
   - raw exception, config detail, or internal-state leakage on user-visible surfaces
   - cost-amplifying flows, missing throttles, or shared mutable state
7. Treat old findings as stale unless the current HEAD fails the deterministic gate or the current code still shows the issue directly.
8. When the same issue class appears more than once in the repo, propose or add a repo-local harness check so future reviews stop rediscovering it manually.

## Output Contract

- Start with the deterministic gate result, identifying newly run versus reused evidence and its tested scope/source and log or CI reference.
- If the gate passes and there are no residual findings, say so explicitly.
- If residual findings remain, report only current-HEAD findings with concrete file and line references.
- If review feedback changes code, tests, or their inputs, rerun affected checks and relevant regressions before closing; broaden only when dependency impact warrants it. Preserve valid evidence for unaffected checks.
- Required CI for an exact candidate and current deployment/live-state claims need their own matching evidence. Local PASS or reviewer approval does not establish either.
