## Phase 2: Interview Loop

Repeat until ambiguity is at or below threshold, the hard cap is reached, or the user exits early.

### Step 2a: Generate Next Question

Build the next question from:

- prompt-safe initial idea
- prior Q&A, trimmed or summarized if needed
- current clarity scores per dimension
- locked work structure
- brownfield codebase context, summarized with file/path/symbol citations
- ontology snapshots from previous rounds
- challenge mode if active
- preserved language instruction

If any input is too large, summarize it before generating the question.

Targeting strategy:

1. Identify the active component with the lowest clarity.
2. Identify that component's weakest dimension.
3. If multiple components are tied, rotate away from the last targeted component.
4. Generate a question that specifically improves that component and dimension.
5. Explain why that component/dimension is the bottleneck.
6. Ask a question that exposes assumptions, not broad feature wishlists.
7. If the core nouns keep changing, ask an ontology-style question before continuing feature detail.

Question styles by dimension:

| Dimension | Question Style | Example |
| --- | --- | --- |
| Goal clarity | What exactly happens when...? | "When you say manage tasks, what specific action does a user take first?" |
| Constraint clarity | What are the boundaries? | "Should this work offline, or is internet connectivity assumed?" |
| Success criteria | How do we know it works? | "If I showed you the finished result, what would make you say yes, that's it?" |
| Context clarity | How does this fit? | "I found JWT auth middleware in `src/auth/`. Should this extend that path or intentionally diverge?" |
| Scope-fuzzy ontology | What is the core thing? | "You have named Tasks, Projects, and Workspaces. Which one is the core entity?" |

### Step 2a-prime: Auto-Research Greenfield Questions

When the next question is greenfield and explicitly tagged `research: true`, load `auto-research-greenfield.md`.

Pass only:

- the tagged question
- locked work structure summary
- prompt-safe initial idea
- prior decisions/gaps
- relevant constraints

The fragment must return 2-3 ranked candidates with rationale, confidence, and uncertainty. Validate the shape before using it. If it is useful, incorporate the candidates into the single user-facing question. If it fails or is not useful, ask the normal manual question and increment `architect_failures`; do not invent a requirement.

### Step 2b: Ask The Question

Present each question with this structure:

```text
Round {n} | Component: {target_component_name} | Targeting: {weakest_dimension} | Ambiguity: {score}%
Why now: {one_sentence_targeting_rationale}

Current understanding: {one sentence}
Blocked decision: {the decision that cannot be made safely yet}
Recommended answer: {sensible default if one exists; otherwise say no default}
Question: {exactly one question}
```

Translate the whole structure to the user's language except stable labels or code identifiers where preserving them helps.

### Step 2b-prime: Auto-Answer Uncertain Opt-Out

If the user opts out, answers with uncertainty, or explicitly asks the agent to decide, load `auto-answer-uncertain.md`.

Pass only:

- the opted-out question
- transcript summary
- locked work structure
- current scores/gaps
- any auto-research candidates used for the round

The fragment must return exactly one tentative answer with rationale, confidence, and uncertainty. Validate before using.

Auto-answer clarity cap:

- If confidence is not high and uncertainty is not negligible, no dimension improved only by auto-answer may exceed `0.85`.
- If the auto-answer would cross the threshold, ask the user for threshold-crossing confirmation before the final ticket.
- If validation fails, continue treating the gap as unresolved.

### Step 2c: Score Ambiguity

After each user answer, score all clarity dimensions from 0.0 to 1.0.

For greenfield:

```text
ambiguity = 1 - (goal * 0.40 + constraints * 0.30 + criteria * 0.30)
```

For brownfield:

```text
ambiguity = 1 - (goal * 0.35 + constraints * 0.25 + criteria * 0.25 + context * 0.15)
```

Scoring prompt to apply internally:

```text
Given the interview transcript for a {greenfield|brownfield} project, score clarity on each dimension from 0.0 to 1.0.

Honor the locked work structure. Score every active component independently and never drop sibling components just because one component is already clear.

Score each dimension:
1. Goal clarity: Is the primary objective unambiguous? Are core entities and relationships stable?
2. Constraint clarity: Are boundaries, limitations, and non-goals explicit?
3. Success criteria clarity: Could a test or acceptance check verify success?
4. Context clarity for brownfield: Do we understand the existing system well enough to modify it safely?

For each dimension provide:
- score
- one-sentence justification
- gap if score < 0.9

Also identify:
- weakest_component_id
- weakest_dimension
- weakest_dimension_rationale
- component_scores keyed by component id
```

### Ontology Extraction

After each answer, identify key entities:

- name
- type
- fields
- relationships

For rounds after the first, compare with the previous ontology snapshot:

- `stable_entities`: same name and concept
- `changed_entities`: renamed but same type and more than 50% field overlap
- `new_entities`: not matched to a prior entity
- `removed_entities`: no longer present
- `stability_ratio`: `(stable + changed) / total_entities`

Show the user enough matching reasoning to sanity-check the result. Renamed entities are convergence, not instability, when the concept persists.

### Step 2d: Report Progress

After scoring, show:

```text
Round {n} complete.

| Dimension | Score | Weight | Weighted | Gap |
| --- | ---: | ---: | ---: | --- |
| Goal | {s} | {w} | {s*w} | {gap or clear} |
| Constraints | {s} | {w} | {s*w} | {gap or clear} |
| Success Criteria | {s} | {w} | {s*w} | {gap or clear} |
| Context (brownfield) | {s} | {w} | {s*w} | {gap or clear} |
| Ambiguity | | | {score}% | |

Work structure: Targeted {target_component_name} | Active: {active_count} | Deferred: {deferred_count} | Next rotation after: {last_targeted_component_id}
Ontology: {entity_count} entities | Stability: {stability_ratio} | New: {new} | Changed: {changed} | Stable: {stable}
Next target: {target_component_name} / {weakest_dimension} - {weakest_dimension_rationale}
```

If ambiguity is at or below threshold, say the clarity threshold is met and move to Phase 4. Otherwise ask the next Phase 2 question.

### Step 2e: Update State

Track in conversation:

- new round
- global scores
- per-component clarity scores
- per-component weakest dimensions
- ontology snapshot
- last targeted component id
- auto-researched rounds
- auto-answered rounds
- architect failures

### Step 2f: Soft Limits And Exit

- Round 3+: if the user says enough, let's go, build it, or equivalent, allow early exit with a risk warning.
- Round 10: show a soft warning and ask whether to continue or proceed with current clarity.
- Round 20: hard cap. Proceed to a pending-approval ticket with current clarity and explicit risk.
- If ambiguity stalls within plus or minus 0.05 for 3 rounds, activate Ontologist mode.
- If all dimensions are 0.9 or higher, proceed to Phase 4 even if the numeric threshold is not exactly met.
- If codebase exploration fails for brownfield, surface the failure and ask whether to continue with limited context. Do not silently treat it as greenfield.
