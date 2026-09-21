---
name: intent-driven-qa
description: Translate intended product behavior into executable automated test contracts, place each risk at the appropriate unit, module, integration, or end-to-end layer, verify RED-to-GREEN evidence, and audit whether existing tests detect meaningful regressions. Use when asked to design tests from requirements, add regression tests before a fix, implement against a test contract, or assess test effectiveness. Do not use for merely running existing tests, generic code review, manual visual QA, or specialized security, performance, load, or accessibility testing.
metadata:
  short-description: Turn intent into defect-detecting test contracts
---

# Intent-Driven QA

Turn what the user actually wants into an executable contract that guides implementation and catches realistic defects. Preserve the user's language in user-facing questions and reports.

## Scope And Authority

- Respect the requested mode and mutation scope. A request to design or audit tests does not authorize product-code changes.
- Ask only when an unresolved choice would materially change observable behavior. Otherwise state the assumption and continue.
- Inspect repository instructions, test commands, fixtures, and public interfaces before designing tests for an existing codebase.
- Prefer the repository's existing test framework and conventions. Do not add a new framework merely to use this skill.
- Do not silently expand ordinary QA into specialized security, performance, load, accessibility, destructive, or live-production testing.

## Source Of Truth

Resolve expected behavior in this order:

1. The user's stated intent and acceptance criteria.
2. Authoritative product, protocol, or domain specifications supplied or referenced for the task.
3. Repository documentation and established public contracts.
4. Existing tests.
5. Current implementation.

The current implementation is evidence, not the oracle. Do not derive expected values from the behavior being tested, copy the production algorithm into the test, or use the same production helper to calculate both actual and expected results. When these sources conflict, report the conflict and identify the decision needed instead of silently choosing the implementation.

## Choose A Mode

- **Contract only:** produce the intent contract and requirement-to-test matrix without editing files.
- **Tests only:** add or improve tests within the authorized scope, but do not change product code.
- **Test-first implementation:** write the focused test, verify the intended RED, make the minimum product change, then prove GREEN and run relevant regression tests.
- **Regression lock:** reproduce a reported bug through a stable public boundary, verify RED, and fix it only when implementation changes were requested.
- **Suite audit:** determine which realistic defects the existing suite can miss. Coverage percentage alone is not evidence of test strength.

Infer the mode from the user's verbs and stated scope when it is clear. State the selected mode before making changes.

## Build The Intent Contract

Before implementation, record the smallest useful contract:

- desired user-visible outcome;
- explicit non-goals and mutation boundaries;
- normal, error, and relevant boundary behavior;
- state transitions and invariants;
- material assumptions and unresolved decisions;
- an independent oracle for every observable condition.

Consider only risks relevant to the feature, such as time boundaries, ownership and tenant isolation, retries and idempotency, concurrency and atomicity, partial failure, serialization, persistence, ordering, pagination, duplicate delivery, rollback, and recovery. Do not fill a fixed checklist with irrelevant cases.

Assign each contract item a stable ID so requirements, tests, evidence, and residual risks remain traceable.

## Place Tests At The Cheapest Trustworthy Layer

- **Unit:** one rule, calculation, branch, parser, or pure state transition can be proved directly.
- **Module/component:** the public entry point of one package, service, or subsystem must be exercised with its internal collaborators connected and only out-of-process boundaries substituted.
- **Integration:** database semantics, framework wiring, serialization, filesystem behavior, adapters, or protocols are part of the contract.
- **End-to-end/system:** a critical user journey or cross-system connection cannot be established credibly at a lower layer.

Start at the lowest layer that can prove the behavior. Duplicate a scenario at a higher layer only when that layer establishes a distinct wiring, persistence, or trust-boundary risk. Do not create the same test pyramid at every layer by default.

## Reuse Evidence And Retry Only Affected Work

Before running checks, inspect the results already collected by local commands, CI, and other reviewers or skills. Use one evidence record for the task, extending existing logs or notes instead of creating a parallel checklist. Record only what the claim needs: contract or covered scope, tested source/artifact identity, command and relevant configuration, platform/toolchain/dependencies, result, and a retrievable log or run reference. Include the relevant working-tree changes when the source is dirty; a commit ID alone does not identify that code.

- Reuse a recorded execution when its covered behavior and relevant inputs still match. A passing broader suite can supply an included focused result when the test actually ran under equivalent conditions. Re-reading the same result through another skill or Pro is not additional test coverage.
- Invalidate only checks affected by changes to the contract, product code, tests, fixtures, configuration, dependencies, or environment. Explain the affected boundary before rerunning; broaden when shared dependencies or uncertain impact justify it. Do not repeat an unchanged check merely because the workflow reached another stage.
- Required CI for an exact candidate must still exist for that candidate. Reused local evidence is not a new CI run or approval for a different artifact. For publication claims, verify the binding between the reviewed source, CI candidate, and published artifact using existing Git/CI/artifact checks; include content hashes only where needed.
- Static test evidence does not establish current external state. Refresh the specific account, running process, deployment, or live delivery observation when making that claim, without rerunning unrelated static tests.

Classify a failure before choosing what to repeat:

| Failure | Next action | Evidence to retain |
| --- | --- | --- |
| Product behavior or incorrect oracle | Resolve the contract/oracle, make the authorized fix, then run the affected tests and relevant regression scope. | Results outside the affected dependency boundary. |
| Fixture, environment, or runner | Repair the failing setup and rerun the checks it prevented or invalidated; retain assertions and timeout requirements unless the contract justifies changing them. | Unaffected executions in valid environments. |
| Missing, redacted, or inaccessible evidence | Recover the original log or artifact reference and verify its identity through an authorized direct read or deterministic comparison. Rerun only the check whose evidence cannot be recovered. | Code/test results and stage decisions whose inputs and supporting evidence remain valid. |

For example, a reviewer unable to compare masked commit IDs has an evidence gap, not proof of a product defect. Use an existing read-only comparison of the exact artifacts where possible; do not disable secret redaction or rebuild and rerun the whole suite solely to reformat evidence. If identity cannot be established, label that claim unverified.

Preserve completed stages whose inputs remain valid and resume at the failed stage. Every retry needs a changed input, recovered evidence, or a concrete transient-failure hypothesis and a bounded retry budget. Stop and report the remaining gap if the same failure recurs without progress; do not restart the whole workflow indefinitely. Pro review is optional unless the user or repository requires it. When used, send the changed scope or unresolved question with existing evidence; do not invent a follow-up after a supported answer already resolves the request.

## Test Strength Rules

- Assert observable outcomes and invariants. Assert internal call order or counts only when the interaction itself is the contract.
- Mock external boundaries, not the behavior under test. A mock returning the value configured by the test does not prove the production adapter works.
- Include negative and boundary cases that could change the product decision, not permutations added only to increase counts.
- Control time, randomness, ordering, and network behavior. Prefer a controlled clock or bounded eventual assertion over arbitrary sleeps.
- Use snapshots only for deliberately stable contracts; avoid broad snapshots that hide meaningful changes in noise.
- Make async assertions observable and awaited. Isolate shared state so tests remain deterministic and order-independent.
- Do not make GREEN by adding `skip`, `only`, quarantine, excessive retries, weaker assertions, or implementation-specific expectations.
- For a high-risk contract, use an independent review pass or controlled mutation check when available and within scope to confirm the test fails for a realistic defect.

## RED To GREEN Protocol

For new or changed behavior:

1. Freeze the intent contract and focused test before changing product code.
2. Run the smallest command that exercises the new test.
3. Confirm it fails because the intended contract is missing or broken, not because of syntax, import, fixture, environment, or setup failure.
4. Record the command, failing test, violated contract ID, and intended failure reason.
5. Make the minimum authorized product change without weakening the contract.
6. Run the same focused test and record GREEN.
7. Run the relevant module or regression suite and record its result.

These steps specify required evidence, not duplicate executions. Reuse valid recorded RED, GREEN, or regression runs under the rules above; do not replay a completed sequence solely for another reviewer.

If the new test is GREEN before implementation, say so. Classify it as characterization or added coverage; never invent RED evidence. When a controlled mutation would add meaningful confidence, perform it only in an isolated or safely reversible workspace and restore it before continuing.

If a test must change after implementation starts, classify the reason first: changed requirement, incorrect oracle, or broken fixture/setup. Record the reason, update the contract when necessary, and re-establish meaningful failure evidence rather than quietly editing the assertion to match the code.

## Suite Audit

Look for tests that can pass despite a realistic defect:

- status-only or "does not raise" assertions with no business outcome;
- expected values computed by the implementation under test;
- mocks that bypass the adapter or protocol risk they claim to verify;
- happy paths without relevant permission, state, failure, or boundary behavior;
- broad snapshots, unawaited async assertions, timing flakes, leaked shared state, or misleading test names.

For each material gap, report the realistic escaped defect, affected contract, observed evidence, cheapest trustworthy test layer, and proposed test. If changes are authorized, prefer adding the focused defect-detecting test over writing a speculative checklist.

## Required Result

Report proportionally to the task, using this structure when implementation or an audit is involved:

```text
Mode and scope
Intent contract
- desired outcome
- non-goals
- assumptions or open decisions

Requirement-to-test matrix
| ID | Observable contract | Risk | Layer | Test and oracle | Status |

Evidence
- RED: command, failing test, intended reason
- GREEN: same focused test
- Regression: relevant suite result
- For each result: newly run or reused, tested identity, environment, and log/run reference
- Reruns: invalidated evidence and the change or gap that required another execution

Changes
- test files
- product files, only when authorized
- contract or test changes and rationale

Residual risk
- unautomated or environment-blocked behavior and why it remains
```

Report PASS only with actual execution evidence, and identify reused results rather than claiming a new run. Distinguish failed behavior, blocked or unverified evidence, static inference, and remaining risk. Keep code QA, reviewer approval, publication, and running-runtime verification separate; one does not imply the others.

## Definition Of Done

Apply only the items relevant to the selected mode. Contract-only work is complete with a traceable matrix and explicit `not run` evidence status; it does not require artificial RED or GREEN runs.

- Every in-scope contract item maps to a test or an explicit reason it remains unautomated.
- New behavior has intended RED evidence, or the absence of RED is reported honestly.
- The same focused test is GREEN after the authorized implementation.
- Relevant regression tests pass.
- No test was disabled or weakened merely to obtain GREEN.
- Remaining assumptions, environment gaps, and risks are visible.
