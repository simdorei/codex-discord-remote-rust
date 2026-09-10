## Phase 3: Challenge Modes

At specific round thresholds, shift the perspective. Each mode is used once.

### Round 4+: Contrarian Mode

Inject this into question generation:

```text
You are now in CONTRARIAN mode. Your next question should challenge the user's core assumption. Ask "What if the opposite were true?" or "What if this constraint does not actually exist?" The goal is to test whether the framing is correct or habitual.
```

### Round 6+: Simplifier Mode

Inject:

```text
You are now in SIMPLIFIER mode. Your next question should probe whether complexity can be removed. Ask "What is the simplest version that would still be valuable?" or "Which constraints are actually necessary vs assumed?"
```

### Round 8+: Ontologist Mode

If ambiguity is still above `0.30`, inject:

```text
You are now in ONTOLOGIST mode. The ambiguity is still high, suggesting we may be addressing symptoms instead of the core problem. The tracked entities so far are: {current_entities_summary}. Ask "What IS this, really?" or "Which entity is the CORE concept and which are supporting?"
```

## Phase 4: Crystallize Spec

When ambiguity is at or below threshold, hard cap is reached, or early exit is confirmed:

1. Generate a final spec from the prompt-safe transcript.
2. Preserve the user's language in prose.
3. Keep code identifiers, file paths, commands, JSON keys, settings keys, and source citations unchanged.
4. Do not write the spec to disk unless the user explicitly asked for a file.
5. Mark the spec as pending approval.

Spec structure:

```markdown
# Deep Interview Spec: {title}

## Metadata
- Interview ID: {id if used}
- Rounds: {count}
- Final Ambiguity Score: {score}%
- Type: greenfield | brownfield
- Generated: {timestamp if useful}
- Threshold: {threshold}
- Threshold Source: {threshold_source}
- Initial Context Summarized: yes|no
- Status: PASSED | BELOW_THRESHOLD_EARLY_EXIT
- Auto-Researched Rounds: {auto_researched_rounds}
- Auto-Answered Rounds: {auto_answered_rounds}
- Architect Failures: {architect_failures}

## Clarity Breakdown
| Dimension | Score | Weight | Weighted |
| --- | ---: | ---: | ---: |
| Goal Clarity | {s} | {w} | {s*w} |
| Constraint Clarity | {s} | {w} | {s*w} |
| Success Criteria | {s} | {w} | {s*w} |
| Context Clarity | {s} | {w} | {s*w} |
| Total Clarity | | | {total} |
| Ambiguity | | | {1-total} |

## Work Structure
| Component | Status | Description | Coverage / Deferral Note |
| --- | --- | --- | --- |
| {component.name} | active|deferred | {description} | {coverage or deferral reason} |

## Goal
{crystal-clear goal statement covering every active component}

## Included Scope
- {included scope}

## Excluded Scope / Non-Goals
- {non-goal}

## Constraints
- {constraint}

## Acceptance Criteria
- [ ] {testable criterion}

## Assumptions Exposed And Resolved
| Assumption | Challenge | Resolution |
| --- | --- | --- |
| {assumption} | {how it was questioned} | {decision} |

## Technical Context
{brownfield repo findings with paths/symbols, or greenfield technology constraints}

## Ontology (Key Entities)
| Entity | Type | Fields | Relationships |
| --- | --- | --- | --- |
| {entity.name} | {entity.type} | {fields} | {relationships} |

## Ontology Convergence
| Round | Entity Count | New | Changed | Stable | Stability Ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | {n} | {n} | - | - | - |
| {final} | {n} | {new} | {changed} | {stable} | {ratio}% |

## Interview Transcript
### Round 1
Q: {question}
A: {answer}
Ambiguity: {score}%

## Risks And Open Questions
- {remaining gap}

## Proposed Next Step
{pending approval next step}
```

## Phase 5: Execution Bridge

After the spec is ready, present exactly one next-step question and wait:

```text
Your spec is ready (ambiguity: {score}%). How would you like to proceed?
```

Offer context-appropriate options:

1. Refine plan before implementation (recommended for broad, risky, or multi-component work).
2. Approve implementation from this spec.
3. Continue interviewing to improve clarity.
4. Stop here with the pending ticket.

Do not implement directly from deep-interview. Implementation requires a separate explicit user approval after the ticket is shown.

If the user approves implementation:

- restate the approved scope
- note remaining risks
- then proceed according to the normal Codex workflow

If the user chooses plan refinement:

- produce a patch plan, not code
- wait for approval before implementation
