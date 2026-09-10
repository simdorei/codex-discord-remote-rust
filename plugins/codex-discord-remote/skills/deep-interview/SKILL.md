---
name: deep-interview
description: Use for vague, broad, risky, or under-specified work requests before implementation; runs a Gajae Code style Socratic interview with ambiguity scoring, work-structure locking, ontology tracking, and explicit approval gating before coding.
---

# Deep Interview

This is a Codex-port of the Gajae Code deep-interview workflow. Keep the original intent and phase structure intact:

- expose hidden assumptions through Socratic questioning
- mathematically score ambiguity after each answer
- lock the top-level work structure before depth-first questioning
- gather brownfield codebase facts before asking the user to rediscover them
- stop at a pending-approval ticket before any implementation

Source pattern:
https://github.com/Yeachan-Heo/gajae-code/tree/main/packages/coding-agent/src/defaults/gjc/skills/deep-interview

License notice for the upstream material is kept in `NOTICE.md`.

## Codex Port Contract

The upstream Gajae skill uses GJC-specific state, fragments, and workflow entrypoints. Preserve those behaviors with these Codex equivalents:

- Where upstream says `gjc state write` or `gjc state read`, maintain the interview state in the conversation. Do not edit `.gjc/` or create repo artifacts unless the user explicitly asks for persisted planning files.
- Where upstream says `ask tool`, ask the user directly in the normal Codex conversation. Ask exactly one user-facing question at a time.
- Where upstream says `explore agent`, use read-only repository inspection yourself first. If a multi-agent tool is available and explicitly appropriate, it must remain read-only during the interview.
- Where upstream says `/skill:ralplan`, `/skill:ultragoal`, or `/skill:team`, present the equivalent pending next step and wait for explicit user approval. Do not auto-invoke execution, commits, pushes, PRs, mutation commands, or implementation delegates.
- Where upstream refers to `.gjc/specs/deep-interview-{slug}.md`, produce the same spec shape in the conversation unless the user explicitly requests a file.
- Where upstream says fallback after fragment failure, do not invent requirements. Continue the manual interview path and surface any material failure that affects the user-facing decision.
- If the user is writing Korean, every user-facing question, option, and summary must be Korean. Use "작업 구조" instead of "토폴로지" in Korean user-facing text.

## When To Use

Use this skill when any of these are true:

- The user has a vague idea and wants thorough requirements gathering before execution.
- The user says "deep interview", "interview me", "ask me everything", "don't assume", "make sure you understand", or the Korean equivalent.
- The user wants to avoid "that's not what I meant" outcomes from autonomous execution.
- The task is complex enough that jumping to code would spend time discovering scope instead of implementing.
- The user wants mathematically visible clarity before committing to execution.
- The request is brownfield work but lacks explicit integration points, constraints, or acceptance criteria.

Do not use this skill when:

- The user gives a precise command, exact-reply QA, or simple factual question.
- The user has already provided concrete file paths, function names, acceptance criteria, and explicitly asks to execute.
- The user explicitly says not to ask questions or to skip the interview.
- The task is a small, low-risk change where a normal code edit is clearly the requested path.

If the user says "just do it" while the request is still vague, respect the intent by summarizing a pending-approval ticket with the remaining ambiguity. Do not mutate files during the interview.

## Global Rules

- Ask ONE question at a time. Never batch multiple user-facing questions.
- Preserve the user's language for announcements, work-structure confirmation, options, questions, progress reports, and final ticket.
- Target the weakest active component and weakest clarity dimension each round.
- Make weakest-dimension targeting explicit every round: name the dimension, state its score or gap, and explain why the next question is aimed there.
- Before Round 1 ambiguity scoring, run a one-time Round 0 work-structure enumeration gate.
- For brownfield work, gather repository facts before asking the user about codebase context. Cite file paths, symbols, or observed patterns.
- Score ambiguity after every answer and show the score transparently.
- When several active components exist, score and target each component so one detailed component cannot hide unclear siblings.
- Keep prompt payloads budgeted. If the initial context or interview transcript is too large, summarize it before scoring, question generation, or final ticket generation.
- Do not proceed to execution until ambiguity is at or below the resolved threshold and the user explicitly approves a scoped execution path.
- Allow early exit only with a clear warning that names the unresolved gaps.
- Track ontology changes across rounds so shifting nouns or entity names are visible.
- Do not silently fill missing requirements. Turn unknowns into questions.
- If a required read, command, or tool fails, surface the actual error and stop or ask how to proceed.

## Internal Fragments

The upstream workflow has two internal fragments. This port keeps them as files in this skill directory:

- `auto-research-greenfield.md`
- `auto-answer-uncertain.md`

Load them only at the documented hooks:

- Auto-research: between Step 2a and Step 2b when a greenfield question is explicitly tagged `research: true`.
- Auto-answer: after the user opts out, answers with uncertainty, or explicitly asks the agent to decide.

Fragments are internal prompts, not public skills. They must never be advertised as slash commands, registered as separate skills, used to mutate files, or used to skip explicit approval.


## Required References

Before using this skill, read these files completely. They are normative parts of this skill, split only to keep each file focused:

- `references/phase-0-1-work-structure.md` - threshold setup, initialization, and Round 0 work-structure lock.
- `references/phase-2-interview-loop.md` - question generation, auto-research/auto-answer hooks, scoring, ontology, progress, and exits.
- `references/phase-3-5-finalization.md` - challenge modes, final spec, and execution bridge.
- `references/examples-checklist-config.md` - examples, completion checklist, configuration, resume behavior, and score interpretation.

Compatibility markers retained for package checks:

- Phase 0: Resolve Ambiguity Threshold
- Ontology Convergence

Task: {{ARGUMENTS}}
