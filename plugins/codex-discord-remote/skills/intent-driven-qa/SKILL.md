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

Changes
- test files
- product files, only when authorized
- contract or test changes and rationale

Residual risk
- unautomated or environment-blocked behavior and why it remains
```

Never report a check as passing if it was not run. Distinguish verified evidence, static inference, and remaining risk.

## Definition Of Done

Apply only the items relevant to the selected mode. Contract-only work is complete with a traceable matrix and explicit `not run` evidence status; it does not require artificial RED or GREEN runs.

- Every in-scope contract item maps to a test or an explicit reason it remains unautomated.
- New behavior has intended RED evidence, or the absence of RED is reported honestly.
- The same focused test is GREEN after the authorized implementation.
- Relevant regression tests pass.
- No test was disabled or weakened merely to obtain GREEN.
- Remaining assumptions, environment gaps, and risks are visible.
