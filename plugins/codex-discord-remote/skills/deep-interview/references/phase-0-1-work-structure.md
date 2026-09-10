## Phase 0: Resolve Ambiguity Threshold

Complete this phase before initialization, before Round 0, and before any ambiguity score.

1. Resolve the threshold.
   - If the user explicitly gave a threshold, use it.
   - Otherwise use the Gajae default `0.05`.
2. Track the threshold source as one of:
   - `user`
   - `default`
   - `project-config` only if the user explicitly supplied a readable project config path
3. Emit the threshold marker before any other interview announcement:

```text
Deep Interview threshold: {threshold} (source: {threshold_source})
```

4. Carry `threshold` and `threshold_source` into every later score report and the final ticket.
5. Infer the user's language from the request when obvious. Do not surprise a Korean session with English questions.

## Phase 1: Initialize

1. Parse the user's idea from the request.
2. Detect greenfield vs brownfield.
   - Brownfield: the current working directory contains existing source code, package files, or git history, and the request references modifying or extending it.
   - Greenfield: the request is primarily about creating a new thing without existing integration context.
3. For brownfield work, build initial codebase context before Round 1:
   - Inspect relevant files with read-only commands.
   - Search for file names, functions, configuration, command handlers, tests, or domain terms related to the request.
   - Summarize durable facts only: ownership, boundaries, existing patterns, constraints, and unresolved gaps.
   - Do not ask the user to tell you facts the repo already reveals.
4. Normalize oversized initial context:
   - If the request includes large logs, transcripts, screenshots, or file excerpts, summarize them first.
   - Treat the summary as the canonical initial idea.
   - Preserve explicit decisions, constraints, non-goals, file references, and unresolved gaps.
5. Initialize interview state in the conversation:

```json
{
  "active": true,
  "current_phase": "interviewing",
  "interview_id": "{stable id if useful}",
  "type": "greenfield|brownfield",
  "initial_idea": "{prompt-safe idea}",
  "initial_context_summary": "{summary or empty}",
  "rounds": [],
  "current_ambiguity": 1.0,
  "threshold": 0.05,
  "threshold_source": "default",
  "language": "{user/session language}",
  "codebase_context": "{brownfield facts or null}",
  "work_structure": {
    "status": "pending",
    "components": [],
    "deferrals": [],
    "last_targeted_component_id": null
  },
  "challenge_modes_used": [],
  "ontology_snapshots": [],
  "auto_researched_rounds": [],
  "auto_answered_rounds": [],
  "architect_failures": 0
}
```

6. Announce the interview after the threshold marker:

```text
Starting deep interview. I will ask targeted questions before any implementation. After each answer, I will show the clarity score. We proceed only after the ambiguity threshold is met and you explicitly approve the next step.

Your idea: "{initial_idea}"
Project type: {greenfield|brownfield}
Current ambiguity: 100% (not scored yet)
```

Translate this announcement to the user's language.

## Round 0: Work-Structure Enumeration Gate

Run this gate exactly once after Phase 1 initialization and before Phase 2 ambiguity scoring.

The goal is to lock the shape of the user's scope before depth-first questioning overfits to the most-described component.

1. Enumerate candidate top-level components from the initial idea and brownfield context.
   - Extract top-level verbs, nouns, workstreams, surfaces, integrations, or deliverables that can succeed or fail independently.
   - Prefer 1-6 components.
   - If more than 6 appear, group siblings at the highest useful level and explain the grouping.
   - Do not treat implementation tasks, fields, or sub-features as top-level components unless the user framed them as independent outcomes.
2. Ask exactly one confirmation question before scoring:

```text
Round 0 | Work structure confirmation | Ambiguity: not scored yet

I'm reading this as {N} top-level component(s):
1. {component_name}: {one_sentence_description}
2. ...

Current understanding: {one-sentence summary}
Blocked decision: whether these are the right top-level components.
Recommended answer: {sensible default, usually "이 작업 구조로 진행"}
Question: Is this work structure right? Should any component be added, removed, merged, split, or explicitly deferred?
```

For Korean:

```text
Round 0 | 작업 구조 확인 | Ambiguity: not scored yet
...
Question: 이 작업 구조가 맞나요? 추가, 제거, 병합, 분리, 또는 보류할 항목이 있나요?
```

3. After the answer, lock the work structure:

```json
{
  "work_structure": {
    "status": "confirmed",
    "confirmed_at": "{timestamp if useful}",
    "components": [
      {
        "id": "component-slug",
        "name": "Component Name",
        "description": "Confirmed top-level outcome",
        "status": "active|deferred",
        "evidence": ["initial prompt phrase or brownfield citation"],
        "clarity_scores": {
          "goal": null,
          "constraints": null,
          "criteria": null,
          "context": null
        },
        "weakest_dimension": null
      }
    ],
    "deferrals": [
      {
        "component_id": "component-slug",
        "reason": "User-confirmed deferral reason",
        "confirmed_at": "{timestamp if useful}"
      }
    ],
    "last_targeted_component_id": null
  }
}
```

4. Single-component pass-through:
   - If the user confirms one active component, proceed normally while carrying that component into scoring and the final ticket.
5. Multi-component behavior:
   - Every active component must receive sufficient goal, constraint, and success-criteria clarity.
   - Rotate targeting across similarly weak components.
   - The final ticket must cover every active component and explicitly list deferrals.
