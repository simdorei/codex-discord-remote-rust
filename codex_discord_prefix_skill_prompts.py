from __future__ import annotations


DEEP_INTERVIEW_PROMPT_HEADER = """Run a Gajae-style deep interview before implementation.

Follow the packaged deep-interview skill when it is available. If it is not loaded, manually run this Codex port of the Gajae Code workflow.

Codex port rules:
- Do not edit files, run mutation commands, produce implementation code, commit, push, open PRs, or delegate execution yet.
- Preserve the user's language in every user-facing question, option, progress report, and summary.
- Emit the threshold marker first: `Deep Interview threshold: 0.05 (source: default)` unless the user explicitly gave another threshold.
- Round 0: enumerate 1-6 top-level components or outcomes from the request and ask exactly one work-structure confirmation question before ambiguity scoring.
- If the user is writing Korean, say "작업 구조" instead of "토폴로지" in user-facing text.
- Detect greenfield vs brownfield. For brownfield work, inspect relevant code first and cite files, symbols, or patterns before asking the user about existing-system context.
- Then ask one question at a time, targeting the weakest active component and weakest clarity dimension: goal, constraints, success criteria, and existing-system context for brownfield work.
- Each question must include: Round, Component, Targeting, Ambiguity, Why now, Current understanding, Blocked decision, Recommended answer, Question.
- Do not silently fill missing requirements. Turn unknowns into questions. If a required read/tool fails, surface the actual failure instead of substituting another path.
- After each answer, show clarity scores from 0.0 to 1.0 and calculate ambiguity.
- Use greenfield ambiguity = 1 - (goal * 0.40 + constraints * 0.30 + criteria * 0.30).
- Use brownfield ambiguity = 1 - (goal * 0.35 + constraints * 0.25 + criteria * 0.25 + context * 0.15).
- Track key entities and ontology stability after each round: new, changed, stable, removed, and stability ratio.
- Activate challenge modes when useful: contrarian after round 4, simplifier after round 6, ontologist after round 8 if ambiguity is still above 0.30.
- Continue until goal, work structure, included scope, excluded scope, constraints, acceptance criteria, key entities, technical context, and remaining open questions are explicit.
- When enough detail is gathered, summarize a pending-approval ticket with Metadata, Clarity Breakdown, Work structure, Goal, Included scope, Excluded scope, Constraints, Acceptance criteria, Assumptions exposed and resolved, Technical context, Key entities, Ontology convergence, Risks/open questions, and Proposed next step.
- Stop after the pending-approval ticket and wait for explicit user approval before coding or planning execution.

User request:
"""


def build_deep_interview_prompt(user_request: str) -> str:
    return DEEP_INTERVIEW_PROMPT_HEADER + str(user_request or "").strip()


def build_archive_used_prompt(threshold: str) -> str:
    return "$codex-discord-remote:archive-used " + str(threshold or "").strip()
