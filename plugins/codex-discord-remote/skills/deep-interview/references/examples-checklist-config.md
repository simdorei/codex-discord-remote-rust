## Good Examples

Targeting weakest dimension:

```text
Scores: Goal=0.9, Constraints=0.4, Criteria=0.7
Next question targets Constraints because it is lowest at 0.4:
"You mentioned this should work on mobile. Does that mean native app, responsive web app, or PWA?"
```

Gathering codebase facts before asking:

```text
I found JWT authentication in `src/auth/` using passport.js. For this feature, should we extend that auth middleware or intentionally create a separate authentication flow?
```

Contrarian mode:

```text
Round 5 | Contrarian Mode | Ambiguity: 42%
You said this needs to support 10,000 concurrent users. What if it only needed to handle 100? Would the architecture change fundamentally, or is the 10K number an assumption?
```

Early exit warning:

```text
Current ambiguity is 35% (threshold: 5%). Areas still unclear:
- Success Criteria: 0.5
- Constraints: 0.6

Proceeding may require rework. Continue anyway?
```

Ontology convergence:

```text
Round 3 entities: User, Task, Project
Round 4 entities: User, Task, Project, Tag
Round 5 entities: User, Task, Project, Tag

Ontology has converged: the same 4 entities appeared in two consecutive rounds.
```

## Bad Examples

Batching multiple questions:

```text
What is the target audience? What tech stack? How should auth work? Where is it deployed?
```

Asking about facts the repo reveals:

```text
What database does your project use?
```

Proceeding despite high ambiguity:

```text
Ambiguity is 45% but we have done 5 rounds, so let's build.
```

## Completion Checklist

- [ ] Phase 0 completed before Round 0.
- [ ] First user-visible line was the threshold marker.
- [ ] User language was preserved.
- [ ] Work structure gate completed before ambiguity scoring.
- [ ] Every active component has goal, constraints, criteria, and context scores as applicable.
- [ ] Ambiguity score displayed after every answer.
- [ ] Every round named the weakest component and weakest dimension.
- [ ] Brownfield questions cite repo evidence before asking the user to decide.
- [ ] Oversized context was summarized before scoring or final-ticket generation.
- [ ] Challenge modes activated at the correct thresholds when needed.
- [ ] Ontology snapshots tracked key entities and convergence.
- [ ] Auto fragments were used only at their documented hooks.
- [ ] Final ticket includes work structure, goal, constraints, non-goals, acceptance criteria, technical context, ontology, transcript, risks, and proposed next step.
- [ ] No implementation, mutation command, commit, push, PR, or execution delegation occurred before explicit approval.

## Configuration

Optional Gajae-style settings can be represented by user instruction or an explicitly supplied config:

```json
{
  "deepInterview": {
    "ambiguityThreshold": 0.05,
    "maxRounds": 20,
    "softWarningRounds": 10,
    "minRoundsBeforeExit": 3,
    "enableChallengeModes": true,
    "autoExecuteOnComplete": false,
    "defaultExecutionMode": null,
    "scoringModel": "consistent-low-temperature-reasoning"
  }
}
```

Do not silently read or write config files. If a config path matters, ask for or cite it.

## Resume

If interrupted, resume from the conversation state:

- threshold and source
- project type
- locked work structure
- prior rounds
- latest scores
- ontology snapshots
- remaining gaps
- pending final ticket if already produced

If the state is unavailable, say that explicitly and restart from Round 0 rather than pretending prior answers are known.

## Ambiguity Score Interpretation

| Ambiguity | Meaning | Action |
| ---: | --- | --- |
| 0.0-0.1 | Crystal clear | Proceed to pending ticket |
| At or below threshold | Clear enough | Proceed to pending ticket |
| Slightly above threshold | Minor gaps | Continue targeted questioning |
| Moderate | Significant gaps | Focus weakest dimensions |
| High | Very unclear | Consider challenge/ontology mode |
| Extreme | Almost nothing known | Continue early rounds |
