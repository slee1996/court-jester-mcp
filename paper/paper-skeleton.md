# Paper Skeleton

## Working title
Concrete Verifier Feedback Improves Agent Repair Loops Without Obvious Precision Collapse

## Title alternatives
- Court Jester: Verifier-Triggered Repair for AI Coding Agents
- Concrete Counterexamples for Agentic Code Repair

## Abstract
Draft from `paper/court-jester-paper-memo.md`, then tighten once control experiments land.

## 1. Introduction

### Paragraph 1: problem
AI coding agents often stop at plausible code rather than correct code. This is especially costly on semantic repair tasks where failures survive obvious checks and only appear under edge cases, cross-file interactions, or hidden behavior constraints.

### Paragraph 2: why current loops are weak
One-shot editing and thin public checks do not reliably disprove plausible-but-wrong patches. What is often missing is a concrete counterexample at the moment the model is about to claim success.

### Paragraph 3: approach
We present Court Jester, an agent-facing CLI verifier that runs parsing, project-context linting, sandboxed execution, and optional tests, then returns structured failure artifacts that can trigger another repair attempt.

### Paragraph 4: contribution summary
We evaluate whether verifier-triggered repair improves final task success without paying for the gain through excessive false positives.

### Contribution bullets
- We present Court Jester, a verifier designed for agent repair loops rather than human-only inspection.
- We introduce a benchmark methodology that separates utility, precision, and repair attribution.
- We show that verifier-guided repair reaches `230/234` on the primary causal matrix, beating blind retry at `216/234`, public repair at `205/234`, and baseline at `208/234`.
- We show that the same ranking survives both a public-repair proving ground (`25/36` vs `19/36` vs `14/36` vs `11/36`) and a two-step robustness rerun (`156/156` vs `150/156` vs `140/156` vs `137/156`).
- We show that the tightened verifier remains clean on a `270/270` false-positive gauntlet across known-good and upstream replay controls.

### Intro caveat paragraph
The paper does not claim broad arbitrary-repo validity. It studies verifier-triggered repair on curated semantic code tasks and precision controls designed to stress the failure modes most relevant to agent loops.

## 2. Court Jester

### 2.1 Tool overview
- CLI commands: `analyze`, `lint`, `execute`, `verify`
- focus this paper on `verify`

### 2.2 Verify pipeline
- parse
- optional complexity gate
- project-context lint
- synthesized execute stage in sandbox
- optional authoritative test file

### 2.3 Repair-loop interface
Explain why structured failing repros are the main product surface.

### Figure 1
System / loop diagram:
agent edit -> `court-jester verify` -> concrete repro or pass -> repair attempt or finish

## 3. Benchmark Methodology

### 3.1 Product question
Does Court Jester improve final task success in an agent loop without introducing enough false positives to make it net harmful?

### 3.2 Suite roles
- `headline_curated`: `core-current`
- `false_positive_control`: `known-good-corpus`
- `external_false_positive_control`: `external-known-good-replay`
- optional: `verify_mutation_recall` as recall-only, not headline utility

### 3.3 Policies
- `baseline`
- `public-repair-1`
- `retry-once-no-verify`
- `repair-loop-verify-only`
- robustness add-ons for final paper: `public-repair-2`, `retry-twice-no-verify`, `repair-loop-verify-only-2`

### 3.4 Units and metrics
- unit of analysis: benchmark cell = task × model × policy × repeat
- primary metric: final task success
- secondary metrics: false-positive rate, repair-trigger attribution, failure categories

### Figure 2
Policy comparison diagram showing what can trigger another attempt under each policy: baseline, public repair, blind retry, and verify-only.

## 4. Main Results

### 4.1 Utility and causal comparison
Use a table like:

| Model | Baseline | Public repair | Blind retry | Verify-only |
|------|----------|---------------|-------------|-------------|
| claude-default | 101/117 | 98/117 | 108/117 | 115/117 |
| codex-default | 107/117 | 107/117 | 108/117 | 115/117 |
| aggregate | 208/234 | 205/234 | 216/234 | 230/234 |

State clearly:
- verify-only reached `98.3%`
- blind retry reached `92.3%`
- public repair reached `87.6%`
- baseline reached `88.9%`
- verify-only beat blind retry by `+14` successes and public repair by `+25`

### 4.2 Precision
Use a table like:

| Control suite | Result |
|--------------|--------|
| local known-good | 80/80 |
| external replay | 190/190 |
| combined gauntlet | 270/270 |

### 4.3 Residual failures under verify-only
- Claude: one `hidden_semantic_miss`, one `public_check_failure`
- Codex: two `verify_caught_hidden_bug`
- explain these are residual misses, not proof of saturation

### 4.4 Interpretation
- verify-only beat both matched controls
- public repair underperformed baseline overall
- the useful ingredient appears to be concrete verifier-generated repros, not merely another attempt

## 5. Control Experiments and Robustness

### 5.1 Public-repair control
Question: how much of the gain can be reproduced by visible public checks alone?
Result: in the primary causal matrix, public repair reached `205/234`, below both baseline and verify-only.

### 5.2 Blind retry control
Question: is the lift just extra search budget?
Result: in the primary causal matrix, blind retry reached `216/234`, below verify-only at `230/234`.

### 5.3 Proving-ground mechanism result
Question: is public repair only losing on the headline suite because it rarely gets a real chance to fire?
Result: on the proving-ground suite, public repair improved over baseline (`14/36` vs `11/36`), so it was a fair live comparator, but verify-only still won clearly at `25/36`.

### 5.4 Two-step robustness result
Use a table like:

| Policy | Result |
|------|--------|
| baseline | 137/156 |
| public-repair-2 | 140/156 |
| retry-twice-no-verify | 150/156 |
| verify-only-2 | 156/156 |

Then break out by model:
- Claude: `67/78`, `66/78`, `75/78`, `78/78`
- Codex: `70/78`, `74/78`, `75/78`, `78/78`

Interpretation: more budget helped the controls, but verify-only-2 still finished best on both models.

### 5.5 Remaining robustness scope
- two-step proving-ground matrix: [PENDING RUN or EXPLICITLY SCOPED OUT]

### 5.6 Breakdown analyses
- by model family
- by task family
- by language
- by failure type

## 6. Discussion

### Main takeaway
Concrete verifier-generated repros appear to improve repair-loop outcomes on the current benchmarked semantic task pool.

### Why this matters
The value is not only bug detection. It is closing the premature-success gap in agent loops.

### Boundaries
- curated tasks
- current languages limited to Python and TypeScript
- not a CI replacement
- not evidence of arbitrary-repo readiness

## 7. Limitations
- benchmark remains curated
- two-step proving-ground robustness is still missing unless explicitly scoped out
- limited external validity
- no claim of universal effectiveness across all model tiers or coding environments

## 8. Related Work
Organize by theme, not by paper list.

### 8.1 LLM code repair and self-correction [CITATION NEEDED]
### 8.2 Execution-based verification and testing for generated code [CITATION NEEDED]
### 8.3 Counterexample-guided synthesis / repair analogs [CITATION NEEDED]
### 8.4 Agent benchmarking methodology for code tasks [CITATION NEEDED]

## 9. Conclusion
Court Jester offers evidence for a narrower but useful claim: verifier-generated concrete counterexamples can improve coding-agent repair loops on semantic tasks without obvious precision collapse on the current controls.

## Appendix plan
- benchmark task details
- policy JSON snippets
- failure taxonomy table
- additional per-task results
- statistical methodology
- prompts for repair-loop policies

## Figures to build
See `paper/figure-plan.md`.
1. Main four-way causal bar chart: baseline vs public repair vs blind retry vs verify-only
2. Precision summary chart
3. Proving-ground comparison chart
4. Two-step robustness by model
5. Residual failure figure or stacked breakdown

## Tables to build
1. Main results table
2. Precision controls table
3. Failure taxonomy table
4. Control-policy comparison table
