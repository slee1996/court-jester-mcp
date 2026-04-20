# Submission Readiness Audit

## Status
Current status: strong benchmark package with the primary causal matrix complete, but not fully submission-ready.

## Recommended target order
1. Workshop / strong internal memo now
2. TMLR / rigorous benchmark paper after control experiments
3. Major conference only if the causal and statistical story gets materially tighter

## Core question
Can this repo support a paper that survives skeptical review?

Answer: yes, but only if the next round closes the obvious attribution and rigor gaps.

## Current assets

### Proven assets already in repo
- Primary causal matrix with clear aggregate ordering: baseline `208 / 234`, public repair `205 / 234`, blind retry `216 / 234`, verify-only `230 / 234`
- By-model causal lift on both families: Claude `101 / 117 -> 115 / 117`, Codex `107 / 117 -> 115 / 117` under verify-only
- Clean false-positive controls: `270 / 270`
- Methodology docs that already distinguish utility from precision and separate verifier behavior from harness behavior

### Structural assets
- Named task suites with explicit suite roles
- Explicit policy variants in `bench/policies/`
- Existing benchmark methodology doc
- Existing release-note-quality summaries that can be converted into paper prose

## Blocking gaps

### Blocker 1: One-step causal attribution is now strong; two-step robustness is still incomplete
Severity: medium-high

The headline causal question now has two good answers.

Primary causal matrix:
- verify-only beat blind retry (`230 / 234` vs `216 / 234`)
- verify-only beat public repair (`230 / 234` vs `205 / 234`)
- public repair was slightly worse than baseline (`205 / 234` vs `208 / 234`)

Proving-ground mechanism matrix:
- baseline: `11 / 36`
- public repair: `14 / 36`
- blind retry: `19 / 36`
- verify-only: `25 / 36`

That matters because public repair does improve over baseline on the suite designed to favor it, so it is a fair live comparator. Verify-only still wins clearly.

What remains is robustness breadth, not primary attribution:
- the two-step `core-current` matrix is now complete and preserves the same ranking
- ideally run the two-step proving-ground matrix too, unless you intentionally scope the paper around the completed package

### Blocker 2: Statistics and failure-analysis package now needs conversion into final paper tables and figures
Severity: high

The result is now quantitatively strong, and a first paper-ready package has been drafted in `paper/statistics-and-failure-package.md`.

Still needed:
- convert the documented Wilson intervals and deltas into final paper tables
- extract full failure counts for all main policies from run artifacts
- decide whether to add bootstrap or paired task-level analyses on top of the current descriptive intervals
- make the unit of analysis explicit in the paper text

### Blocker 3: Limited external validity framing
Severity: high

The repo is unusually honest about its limits. Good. The paper should keep that honesty.

Need:
- explicit scope statement in intro and limitations
- avoid arbitrary-repo generalization language
- position the contribution as curated semantic repair benchmarking plus verifier-triggered repair behavior

### Blocker 4: Related-work pass is drafted but not yet integrated into the paper manuscript
Severity: medium-high

A first verified pass now exists in:
- `paper/related-work-notes.md`
- `paper/related-work-draft.md`

Still needed:
- integrate the draft into the actual paper manuscript
- optionally strengthen with 1-3 additional citations if they materially sharpen the benchmark or agent-loop framing
- decide whether the final venue wants a more software-engineering bibliography or a more ML-agent bibliography

### Blocker 5: Missing failure taxonomy table
Severity: medium-high

The repo already mentions failure categories like:
- `hidden_semantic_miss`
- `verify_caught_hidden_bug`
- `public_check_failure`
- provider/infrastructure failures in older runs

A paper needs a clean table showing:
- baseline failure mix
- verify-only failure mix
- what errors were converted into wins
- what residual errors remain

### Blocker 6: No paper figures yet
Severity: medium

Minimum viable figure set:
1. Main aggregate bar chart: baseline vs verify-only
2. Precision chart: known-good, replay, combined gauntlet
3. Benchmark loop diagram: baseline vs verify-only vs control policies
4. Optional: failure-conversion Sankey or stacked bar chart

## Submission-grade outline of experiments

### Required experiment set
1. `baseline`
2. `repair-loop-verify-only`
3. `public-repair-1` or `public-repair-2`
4. `retry-once-no-verify` or `retry-twice-no-verify`

### Strongly recommended additions
5. breakdown by language: Python vs TypeScript
6. breakdown by task family: canonicalization, semver, cross-file contract, framework slices
7. failure-kind conversion table
8. ablation of stricter vs older verifier if historical artifacts can be reconstructed cleanly

## Reviewer simulation

### Likely accept-side argument
- The paper asks a practical and important product question.
- The benchmark methodology is unusually explicit about precision versus utility.
- The repeated utility lift is large.
- The false-positive controls are stronger than the usual thin benchmark story.

### Likely reject-side argument
- The benchmark is still curated and internal.
- The paper still lacks paper-grade statistics and failure-analysis tables.
- The current result may overstate generality beyond the completed benchmark package.
- Evidence is benchmark-strong, but external validity remains limited.

## Decision rule
This becomes submission-ready when all of the following are true:
- [x] headline package includes at least one public-repair control
- [x] headline package includes at least one no-verify retry control
- [ ] uncertainty/statistics are computed and reported clearly
- [ ] failure taxonomy table exists
- [ ] related work section exists with verified citations
- [ ] figures exist and are paper-ready
- [ ] intro and limitations are written in scoped, non-inflated language
- [x] two-step `core-current` robustness package is complete
- [ ] two-step proving-ground robustness package is complete or intentionally scoped out

## Best current framing
If forced to submit soon, frame the paper as:
"a benchmarked systems study of verifier-triggered repair for agentic coding"

Do not frame it as:
- general agent correctness
- universal verifier effectiveness
- broad external-repo validation

## Bottom line
This is now a real paper result. Precision controls are clean, the main causal matrix is favorable, the proving-ground matrix is favorable, and the two-step `core-current` robustness matrix is favorable. The remaining gap is no longer whether the control story holds. It is whether the paper packages the result with enough statistics, failure analysis, figures, and related work to survive skeptical review.
