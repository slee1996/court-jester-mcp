# PRD: Behavioral Test Quality

**Status:** Implemented as a stable advisory feature in Court Jester 0.2.16  
**Date:** 2026-08-30  
**Release:** 0.2.16  
**Product surface:** `court-jester verify`, `court-jester ci`, report schema  
**Enforcement:** Advisory; no score, threshold, verdict change, exit-code change, or CI gate

## 1. Decision

Court Jester 0.2.16 promotes the bounded test-quality spike into a stable advisory feature for both direct verification and CI. This PRD remains the implementation and interpretation contract for that release.

The feature combines two independent observations:

1. **Behavior sensitivity:** make a small, validated change behind a required public surface and rerun the authoritative test. A test that reaches the changed surface but still passes has not demonstrated that it observes that behavioral distinction.
2. **Implementation coupling:** statically identify high-confidence cases where the test reaches through the target's public boundary into private members, private imports, private spies, or source/runtime introspection.

Neither observation alone proves that a test is good or bad. The product must report evidence, not a quality score. The first production release must not change `verdict`, `strength`, the process exit code, or CI gate results because of the `test_quality` stage.

The baseline authoritative `test` stage retains its existing semantics. A real baseline test failure may still fail or make verification inconclusive; only the additional quality observations are non-gating.

## 2. Problem

Coverage establishes that a test reached code. It does not establish that the test observed an externally meaningful result. Static test-smell rules can detect some coupling, but cannot establish whether the test would detect a behavioral regression.

This leaves two common failure modes in agent-written unit tests:

- **Reach without observation:** the test calls the changed public API, then asserts a constant, an unrelated value, or only the absence of a crash.
- **Implementation coupling:** the test passes because it asserts a private field, spies on an internal helper, imports a non-public symbol, or inspects source text. Such a test can fail under a behavior-preserving refactor.

Court Jester already owns the boundaries needed to address this in the edit loop: changed-surface analysis, target-entry instrumentation, authoritative native test execution, sandbox limits, and machine-readable evidence. A bounded mutation campaign can reuse those boundaries without becoming a full-repository mutation-testing platform.

## 3. Product thesis

For a changed public surface, the most useful fast answer is not "what percentage of all possible mutants did this suite kill?" It is:

- Which concrete, behavior-changing distinctions did the authoritative test detect?
- Which reached distinctions survived?
- Which observations could not be judged, and why?
- Does the test visibly depend on the target's internals?

A small deterministic campaign with exact target-entry evidence is a good fit for Court Jester's agent repair loop. Full-suite mutation scoring, historical dashboards, exhaustive operator sets, and repository-wide optimization remain the domain of dedicated mutation-testing systems.

## 4. Users and jobs

### Primary user

An AI coding agent that has edited Python or TypeScript and is about to declare the change complete.

**Job:** Find a concrete missing behavioral assertion quickly enough to repair it in the same loop.

### Secondary user

A maintainer reviewing a pull request in CI.

**Job:** See whether tests covering changed public surfaces are sensitive to representative behavioral faults and whether those tests are coupled to target internals.

### Tertiary user

A repository owner evaluating test quality over time.

**Job:** Consume stable per-finding evidence from persisted reports. V1 does not provide historical scoring or trend dashboards.

## 5. Goals

1. Distinguish reached-but-unobserved behavior from behavior that an authoritative test detects.
2. Separate behavior sensitivity from implementation-coupling evidence.
3. Produce a concrete repair witness for each surviving mutant.
4. Reuse the same authoritative runner, project context, runtime profile, limits, and instrumentation as the baseline test.
5. Bound work by an explicit mutation budget and deterministic candidate selection.
6. Support direct `verify` and changed-file `ci` in the first stable release.
7. Preserve the workspace: mutations execute only through temporary overlays.
8. Remain machine-consumable in full and minimal JSON reports and useful in human summaries.

## 6. Non-goals

V1 will not:

- prove that a test is behavior-focused or implementation-independent;
- emit a test-quality grade, percentage, mutation score, or pass threshold;
- replace StrykerJS, mutmut, PIT, or other full mutation-testing platforms;
- mutate an entire repository or exhaust every mutation site;
- automatically generate or commit repaired tests;
- classify all mocks as coupling;
- flag an underscore-prefixed member unless target binding resolution establishes that it belongs to the target;
- generate and execute behavior-preserving refactors to validate coupling findings;
- cache mutant outcomes across commits;
- run mutants concurrently;
- infer arbitrary source-to-test mappings in CI;
- support languages beyond the existing Python and TypeScript/TSX analysis boundary.

## 7. Historical observations from the spike

### 7.1 Historical controlled-matrix observation

During discovery, temporary Python and TypeScript fixtures compared three test styles against three target variants:

| Target variant | Reach-only test | Public-behavior test | Implementation-coupled test |
|---|---|---|---|
| Baseline | Passed | Passed | Passed |
| Behavior-changing mutant | Passed | Failed by assertion | Failed by assertion |
| Behavior-preserving refactor | Passed | Passed | Failed or became inconclusive |

Observed hypotheses:

- a reach-only test survived a behavior change;
- a public-behavior test killed the same change;
- an implementation-coupled test killed the change but broke on an equivalent refactor.

The table records the qualitative observation that motivated the feature. The historical repeat counts and timings were not retained in a committed, reproducible test-quality benchmark artifact, so they are not release evidence or a performance claim.

### 7.2 Maintained-target probes

Informal probes against local and maintained open-source Python and TypeScript targets also found both killed mutants and a survivor that became killed after adding a public zero-size boundary assertion.

The probes also exposed product gaps rather than weak-test findings:

- native runner incompatibility can block valid test suites;
- target-entry instrumentation can hit sandbox CPU limits before the test budget;
- framework output and worker flags require runner-specific adapters;
- a target-originated exception after entry must be distinguished from infrastructure failure.

These historical observations justified productizing an advisory feature. The repository does not currently contain a committed test-quality-specific matrix, classification oracle, or result bundle that reproduces them; future enforcement or broad precision claims require that evidence first.

### 7.3 Research basis

Mutation testing is a useful proxy for fault detection, but not proof of test quality. Just et al. found a statistically significant correlation between mutant detection and real-fault detection independent of code coverage, while also documenting inherent limitations [1].

Established tools reinforce three design choices:

- StrykerJS distinguishes `Survived` from `NoCoverage` using coverage analysis; Court Jester must likewise require exact entry into the mutated surface before judging a mutant [2].
- StrykerJS uses per-test coverage and concurrency for throughput, but warns that tests selected this way must run independently and in random order. V1 stays sequential and runs the explicit authoritative entrypoint [2].
- StrykerJS incremental mode persists results but cannot automatically recognize every environmental change. V1 does not cache outcomes; correctness takes precedence over speed [3].

## 8. Terminology and interpretation

### Required public surface

An exported, non-nested callable selected by the existing diff and invocation rules. With a diff, only changed required surfaces are eligible. Without a diff, all required exported surfaces in the target are eligible.

### Mutation candidate

A source edit produced by a supported AST operator and attributed to one required public surface.

### Behavioral witness

A short description of the input distinction likely to expose the mutation, such as "exercise the boundary where both operands are equal."

### Outcome contract

| Outcome | Required evidence | Interpretation |
|---|---|---|
| `killed` | Baseline eligible; mutated source valid; exact mutated surface entered; authoritative test failed | The test detected this seeded behavioral change. Target-originated exceptions count; infrastructure failures do not. |
| `survived` | Baseline eligible; mutated source valid; exact mutated surface entered; authoritative test passed | The reached behavioral distinction was not detected. This is repair evidence, subject to equivalent-mutant review. |
| `no_coverage` | Mutant valid; authoritative test did not enter the exact mutated surface | No conclusion about sensitivity. |
| `invalid` | Mutation could not be applied, parsed, or preserve the required callable surface | Tool/operator defect or unsupported candidate; no conclusion about the test. |
| `blocked` | Instrumentation, runner, timeout, memory, sandbox, or other non-target infrastructure prevented a judgment | Environment or adapter problem; no conclusion about the test. |

`killed + survived` are **judged**. `invalid + blocked + no_coverage` are **unjudged**. Unjudged outcomes must never be folded into kills or a denominator presented as a quality score.

### Coupling interpretation

| Mutation evidence | Coupling evidence | Product message |
|---|---|---|
| Killed | None | Strongest available evidence that the test observes behavior through the public boundary. |
| Killed | Present | Sensitive, but coupled to a target implementation detail that merits review. |
| Survived | None | Reached behavior was not observed; use the witness to add a public assertion. |
| Survived | Present | Missing observation plus visible implementation coupling. |
| Unjudged | Any | Report only what was observed; do not infer behavioral quality. |

## 9. User experience

### 9.1 Direct verify

Release 0.2.16 uses the stable clean-cut CLI surface:

```bash
court-jester verify \
  --file src/pricing.py \
  --language python \
  --project-dir . \
  --test-file tests/test_pricing.py \
  --tests-only \
  --test-quality 8
```

Requirements:

- `--test-quality` is opt-in.
- An omitted value means 8 mutants.
- Valid values are 1 through 32.
- It requires exactly one authoritative `--test-file`.
- The former experimental flag was removed for the stable release; no alias remains.
- `--tests-only` remains optional. Without it, normal parse, lint, coverage, execute, and test behavior is unchanged, followed by `test_quality` when eligible.

### 9.2 CI

CI accepts the same global mutation budget and explicit authoritative test entrypoints:

```bash
court-jester ci \
  --base origin/main \
  --head HEAD \
  --project-dir . \
  --test-file tests/test_all.py \
  --test-file tests/all.test.ts \
  --test-quality 8
```

V1 CI rules:

1. `--test-file` is repeatable in `ci`, with at most one entrypoint per language. Supported Python and TypeScript-family test extensions determine the entrypoint language.
2. The same language-specific entrypoint runs against each changed target that has an eligible mutation candidate. Users may supply a repo-local aggregator file; Court Jester does not infer arbitrary source-to-test mappings.
3. `--test-quality N` is a **global per-command mutant budget**, not `N` per file. Candidates are distributed deterministically across eligible changed public surfaces and files.
4. Baseline authoritative tests run before mutation judgment. A failing baseline retains the normal authoritative-test verdict semantics.
5. Files with no matching test entrypoint, no candidate, unsupported instrumentation, or no target entry receive explicit skipped or unjudged evidence. They do not make the quality stage gate CI.
6. CI JSON keeps per-file `test_quality` stages and aggregates `planned`, `killed`, `survived`, `invalid`, `blocked`, `no_coverage`, and `coupling`, plus derived `unjudged`. Human and GitHub summaries may collapse the three unjudged outcomes into `unjudged`; neither form creates a score.
7. `--tests-only` remains unsupported by `ci`; CI continues to run its configured verification gates.

A polyglot PR can therefore supply one Python and one TypeScript entrypoint. Requiring explicit entrypoints is preferable to silently selecting an incomplete suite.

### 9.3 Human output

One-line summary:

```text
test_quality ADVISORY 147 ms killed=3 survived=1 unjudged=0 coupling=1
```
Machine-readable CI JSON still exposes `invalid`, `blocked`, and `no_coverage` separately; only the human-facing summary may collapse them into `unjudged`.

Each survivor must include:

- target file and public surface;
- mutation source line and operator;
- original and replacement source fragment;
- exact target-entry confirmation;
- behavioral witness;
- test outcome and bounded failure excerpt when relevant.

Each coupling finding must include:

- test file location;
- resolved target symbol;
- finding kind;
- source evidence;
- a review-oriented message, not a claim that the test is invalid.

## 10. Functional requirements

### FR-1: Eligibility

The stage runs only when:

- the user opted in;
- an authoritative test entrypoint exists for the target language;
- the baseline test passes under the normal authoritative runner;
- required public surfaces are identified;
- target-entry instrumentation is supported;
- the baseline establishes the existing required-surface coverage contract.

When any precondition fails, emit a skipped or advisory stage with `baseline_eligible: false` and a specific reason. Never silently omit a requested stage.

### FR-2: Conservative mutation operators

V1 supports only the operators validated by the spike:

- comparison boundary: `< ↔ <=`, `> ↔ >=`;
- equality negation: `== ↔ !=`, `=== ↔ !==`;
- selected condition negation when no narrower supported operator exists inside the condition;
- boolean literal inversion.

Operators must be AST-aware and language-valid. Expansion requires new labeled positive, negative, and equivalent-mutant fixtures; operator count is not itself a success metric.

### FR-3: Deterministic planning

For identical source, diff, budget, and tool version, candidate IDs, order, and allocation must be identical.

The planner must:

1. attribute each candidate to the narrowest required public surface;
2. deduplicate identical edits;
3. sort by stable source location and operator order;
4. allocate round-robin across required surfaces;
5. in CI, allocate the global budget round-robin across files and surfaces;
6. never exhaust the budget on the first function merely because it appears first.

### FR-4: Mutation validation

Before execution, every candidate must:

- apply exactly once to the intended byte range;
- parse under the same language/source mode as the baseline;
- preserve every required callable's identity and invocability;
- preserve the target source path and project context through an overlay;
- leave the user's workspace unchanged.

A failed check produces `invalid`, not `survived`, `killed`, or a verifier verdict change.

### FR-5: Authoritative execution reuse

Baseline and mutant runs must share one execution path for:

- runner selection and adapter behavior;
- project and package roots;
- dependency resolution;
- runtime profile;
- network, memory, and timeout limits;
- harness arguments;
- target-entry event parsing;
- failure diagnostics.

A second ad hoc test runner is prohibited.

### FR-6: Exact target-entry requirement

A mutant is judged only if the authoritative test emitted the exact mutated `surface_id`. Module load, sibling callable entry, line coverage elsewhere, or a runner exit code is insufficient.

### FR-7: Outcome classification

Classification must follow the table in Section 8. Precedence is:

1. invalid mutation;
2. instrumentation or non-target infrastructure blocker;
3. no exact mutated-surface entry;
4. passed test -> survived;
5. failed test -> killed.

The report retains `assertion_failure`, target-originated exception evidence, timeout, memory, exit code, duration, and failure excerpt so consumers can audit the classification.

### FR-8: Target-aware coupling analysis

The test AST scan supports these high-confidence kinds:

- `private_target_access`;
- `private_target_import`;
- `private_target_spy`;
- `target_source_introspection`.

The analyzer must resolve import aliases and direct imports to the target source file. It must not:

- flag a private-looking member on an unrelated dependency;
- blanket-flag mocks of documented external boundaries;
- flag a public call merely because the same symbol name exists privately elsewhere;
- infer coupling from text search alone.

Parse or resolution failure is reported separately as `coupling_error` and never becomes a finding.

### FR-9: Reporting contract

The stage name remains `test_quality`. Full JSON includes:

```json
{
  "name": "test_quality",
  "status": "advisory",
  "detail": {
    "experimental": false,
    "mode": "advisory",
    "max_mutants": 8,
    "baseline_eligible": true,
    "counts": {
      "planned": 4,
      "killed": 3,
      "survived": 1,
      "invalid": 0,
      "blocked": 0,
      "no_coverage": 0
    },
    "mutants": [],
    "coupling_findings": [],
    "planning_error": null,
    "coupling_error": null
  }
}
```

Requirements:

- Keep existing spike field names and meanings where they are already correct.
- Keep `experimental: false` in the stable release; do not silently drop the field from schema v3.
- Full and minimal reports retain the stable detail fields, including counts, eligibility, per-mutant evidence, errors, and coupling findings.
- Every coupling finding includes the normalized authoritative `test_source_file`; its location and evidence are attributable to that file rather than inferred from the target path.
- No `score`, `percentage`, `grade`, or synthetic pass/fail field is added.
- Stage status is `passed` only when every planned mutant is killed and there are no coupling findings, unjudged outcomes, mutation-planning diagnostics, or baseline infrastructure blockers. This status is informational and remains non-gating.
- Stage status is `advisory` when any survivor, coupling finding, or unjudged outcome exists, when mutation planning produces a diagnostic, or when a non-target authoritative baseline infrastructure blocker prevents the campaign. `advisory` remains valid when all mutant counts and coupling are zero.
- Stage status is `skipped` only for a blocker-free inability to run a campaign with no coupling finding, including no eligible candidates, no share of the global CI budget, or no matching authoritative test entrypoint.

### FR-10: Repair guidance

A survivor message must name the source change and witness. It must not say that the whole test or suite is weak.

Preferred form:

```text
Test passed after changing `>=` to `>` in `eligible`; exercise the boundary where both operands are equal.
```

### FR-11: CI aggregation

CI derives, without rescoring, a top-level `test_quality` aggregate:

```json
{
  "max_mutants": 8,
  "planned": 8,
  "killed": 6,
  "survived": 1,
  "invalid": 0,
  "blocked": 1,
  "no_coverage": 0,
  "unjudged": 1,
  "coupling": 2
}
```

- `max_mutants` is the configured global per-command budget.
- `planned`, `killed`, `survived`, `invalid`, `blocked`, `no_coverage`, and `coupling` are sums derived exactly from per-file stages.
- `unjudged` is the derived sum `invalid + blocked + no_coverage`.
- Machine-readable JSON must retain all three separate unjudged outcome totals as well as `unjudged`; consumers must not have to reverse an aggregate to distinguish invalid mutations, infrastructure blockers, and missing exact-surface coverage.
- Per-file reports retain their complete `test_quality` stage evidence.

Aggregate planned count must not exceed the global budget, and every aggregate count, including the derived `unjudged`, must agree with the per-file stages.

### FR-12: Bounded execution

- Direct verify and CI accept 1..32 planned mutants.
- In CI, the cap applies to the entire command.
- Mutants execute sequentially in V1.
- Existing per-test timeout, memory, network, and runtime-profile controls apply to every run.
- Budget exhaustion is explicit; it does not imply that unselected sites passed.
- The planner and instrumentation must add no hidden test reruns beyond baseline eligibility and the reported mutant executions.

## 11. Non-functional requirements

### Correctness

- Identical inputs produce identical plans and classifications.
- A tool or infrastructure failure cannot become a kill.
- A mutant not entered cannot become a survivor.
- The feature cannot alter the baseline test result, verifier verdict, strength, or selected CI gates.
- Candidate and baseline workspaces remain byte-identical after execution.

### Performance

- Before any future enforcement proposal, a committed feature benchmark must define and demonstrate an acceptable planning and AST-coupling p95; 250 ms, excluding native test execution, is the current candidate threshold rather than a satisfied 0.2.16 release gate.
- The report must separate planning time from test execution time.
- CI's global mutation cap must hold across any number of changed files.
- No result cache ships in V1.

### Compatibility

The stable advisory implementation supports these compatibility boundaries:

- Python authoritative test files execute directly as scripts; top-level assertions and calls are supported.
- Pytest collection and fixtures are not claimed for 0.2.16.
- TypeScript Bun tests;
- TypeScript Vitest through the supported repository-native path;
- TSX parsing where the target or authoritative test uses TSX;
- local-trusted and isolated runtime profiles where the baseline authoritative runner is supported.

Unsupported runner behavior must be `blocked` with a diagnostic, not misclassified as test weakness.

### Security and isolation

- Mutants use the existing materialization overlay and sandbox controls.
- No network access is added.
- No generated source is written into the repository.
- Failure excerpts use the existing bounded and redacted report path.

## 12. Success metrics and future enforcement gates

### Product metrics

V1 is successful when it produces actionable evidence without degrading baseline verification:

1. **Judgment rate:** `(killed + survived) / planned`, reported by language and runner.
2. **Actionable survivor precision:** proportion of manually reviewed survivors that represent a real missing observable distinction rather than an equivalent or malformed mutant.
3. **Coupling precision:** proportion of manually reviewed coupling findings that resolve to a genuine target-internal dependency.
4. **Baseline parity:** percentage of runs whose baseline test status, verdict, and strength are identical with the feature disabled versus enabled before the advisory stage.
5. **Reproducibility:** classification mismatches across repeated identical runs.
6. **Runtime:** planning time, baseline time, mutant execution time, and total overhead, with p50/p95 by runner.
7. **Repair conversion:** in labeled repair fixtures, proportion of survivors for which adding the witness-based public assertion changes that mutant to killed without adding coupling.

### Future enforcement evidence gates

Release 0.2.16 satisfies the implementation contract for a stable advisory surface: direct and CI entrypoints, deterministic bounded allocation, unchanged baseline semantics, stable report fields, and non-gating summaries. It does not claim that a test-quality benchmark matrix or quantitative corpus is committed or that the criteria below are mechanically enforced. Before any enforcement proposal or broad precision claim, a reproducible evidence bundle must establish:

- 100% baseline parity across a committed compatibility matrix created for that evaluation.
- Zero classification mismatches across 20 repeated runs per canonical Python and TypeScript fixture in that matrix.
- At least 90% actionable-survivor precision on a manually labeled corpus of at least 50 judged survivors spanning both languages and all supported operators.
- At least 95% precision on at least 50 target-aware coupling findings, including unrelated-dependency negative controls.
- At most 2% `invalid` outcomes on a committed eligible-candidate corpus.
- Every `blocked` outcome names a diagnostic component and reason.
- Direct verify and CI both enforce the configured mutation cap.
- Full and minimal report fixtures validate against the maintained report contract.
- Human, JSON, and GitHub CI summaries agree with per-file evidence.
- The compatibility matrix passes in local-trusted mode; isolated-mode gaps are explicitly documented and never presented as test-quality findings.

The historical spike was below these corpus sizes and left no committed test-quality result artifact. It supported investment in an advisory surface, not an enforcement rollout or a general precision guarantee.

## 13. Future enforcement evidence plan

The following work is required before proposing enforcement. It is not a description of artifacts committed for 0.2.16, and the stable advisory release does not depend on claiming these steps are already complete:

1. Promote the controlled 3x3 Python and TypeScript matrices from temporary discovery artifacts into committed integration fixtures.
2. Add one positive and one equivalent/unsupported negative fixture for every mutation operator in both languages.
3. Add target-binding negative controls for external private members, same-named imports, documented external mocks, and public spies.
4. Add CI fixtures with:
   - one Python file;
   - one TypeScript file;
   - multiple changed files sharing one entrypoint;
   - a polyglot diff with one entrypoint per language;
   - more candidates than the global budget;
   - an uncovered changed file;
   - a baseline test failure;
   - an infrastructure blocker.
5. Add maintained-repository snapshots pinned to immutable revisions. Record runner, dependency install, command, expected baseline, expected classification, and timing.
6. Include at least one known equivalent mutant per operator family so survivor precision is measured rather than assumed.
7. Run 20-repeat stability checks and report mismatch counts.
8. Run the normal Rust integration suite, CLI smoke path, and report-schema fixtures alongside the new test-quality-specific changed-surface benchmark once that benchmark exists.

Every benchmark observation must be one of `killed`, `survived`, `invalid`, `blocked`, or `no_coverage`. Benchmark harness errors and missing evidence are abstentions, not successes.

## 14. Rollout

### Phase 0: Advisory implementation hardening — completed before 0.2.16

- The discovery implementation used an experimental flag and direct `verify` only.
- Known runner and sandbox classification gaps needed by the bounded advisory path were repaired.
- Command-local typed configuration replaced the temporary environment-variable bridge.

Exit satisfied for the implemented advisory behavior and report contract. No committed matrix, classification oracle, result bundle, or mechanically enforced quantitative threshold is claimed.

### Phase 1: CI advisory implementation — completed before 0.2.16

- Explicit language-specific CI test entrypoints were added.
- The mutation budget became global across changed files.
- Per-file and aggregate summaries were added.
- Quality evidence remained advisory.

Exit satisfied for the implemented CI baseline semantics, global cap, aggregation, and non-gating behavior. The compatibility and precision matrices in Sections 12 and 13 remain future evidence requirements.

### Phase 2: Stable advisory release — shipped in 0.2.16

- The stable CLI clean-cut to `--test-quality` was completed with no compatibility alias.
- Reports preserve the schema-v3 field with `experimental: false`.
- User documentation covers repair evidence, limitations, direct verify, and CI.
- Direct verify and CI are supported.
- No gate, score, or threshold was added.

Exit satisfied for the stable advisory implementation. The quantitative evidence criteria in Section 12 remain required before any future enforcement proposal or broad precision claim.

### Phase 3: Enforcement evaluation, not committed scope

Consider an explicit opt-in gate only after stable advisory evidence demonstrates low false-positive rates in real repositories. Coupling findings must remain review evidence even if mutant survivors later become gateable. A gate proposal requires a separate PRD because it changes verifier verdict semantics.

## 15. Risks and mitigations

| Risk | Consequence | Mitigation |
|---|---|---|
| Equivalent mutant reported as survived | False missing-test guidance | Conservative operators and witness evidence in the advisory release; equivalent-mutant fixtures are required for any future enforcement evidence. |
| Runner failure reported as killed | False confidence | Classification precedence and non-target blocker detection now; a baseline-parity corpus is required before enforcement. |
| Mutant reaches sibling code but not its target | False survivor/kill | Require exact `surface_id` entry. |
| Static coupling false positive | Noise and brittle policy | Target binding and alias resolution now; unrelated-dependency controls are required before enforcement. |
| Full-suite runtime explosion in CI | Feature becomes unusable | Global command budget, sequential bounded runs, explicit entrypoint, timing breakdown. |
| Flaky tests produce inconsistent outcomes | Unreliable evidence | Deterministic planning and no concurrency now; a repeated reproducibility benchmark is required before enforcement. |
| Mutation changes public shape | Invalid product conclusion | Reparse and verify required callable identity before execution. |
| Workspace mutation leaks | User-code corruption | Overlay-only execution; byte-identity regression evidence is required before enforcement. |
| Advisory stage mistaken for proof | Overclaiming | Evidence language, no grade, explicit unjudged outcomes and limitations. |
| CI test selection misses relevant tests | Low judgment rate | Require explicit entrypoints; report no coverage; do not infer arbitrary mappings in V1. |
| Schema drift breaks consumers | Integration regressions | Preserve existing stage and field names; update full/minimal fixtures and schema docs atomically. |

## 16. Alternatives considered

### Coverage-only enforcement

Rejected. Exact entry is necessary but only proves reach. In the temporary discovery fixtures, reach-only tests passed while a reached behavior-changing mutant survived; this remains a motivating observation rather than release benchmark evidence.

### Static test-smell linting only

Rejected as the primary mechanism. It can identify selected coupling patterns but cannot show whether a test observes a public behavior change. Retained as the second, independent signal.

### Full mutation score

Rejected. A bounded changed-surface sample has no stable exhaustive denominator, equivalent mutants distort the percentage, and a score hides invalid, blocked, and uncovered evidence.

### Integrate a full external mutation engine immediately

Deferred. StrykerJS and mutmut offer broader operators and mature optimization, but introduce separate configuration, runner, sandbox, report, and dependency boundaries. Court Jester's differentiated value is a bounded, target-entry-aware repair loop. External-engine adapters may be reconsidered after the native contract is stable.

### LLM review of test intent

Rejected as authoritative evidence. It may help explain a finding later, but is non-deterministic and cannot replace an executed counterexample.

### Behavior-preserving refactor campaign

Deferred. It is a strong way to validate implementation coupling, as the spike showed, but automatically proving refactor equivalence is a separate hard problem. V1 uses high-confidence target-aware static findings and keeps them advisory.

### Cached or incremental mutation outcomes

Deferred. Invalidation must account for source, tests, dependencies, configuration, environment, and runner changes. Recomputing a bounded campaign is safer for V1.

## 17. Resolved product decisions

- Initial enforcement is advisory.
- V1 includes direct verify and CI.
- CI uses explicit test entrypoints rather than inferred source-to-test mappings.
- The mutation budget is global per command.
- Findings remain evidence; no score or threshold ships.
- The stable flag is `--test-quality` with a clean cutover from the experimental name.
- The existing `test_quality` report stage and outcome vocabulary remain stable.
- No gating behavior is included in this PRD.

## 18. References

1. René Just, Darioush Jalali, Laura Inozemtseva, Michael D. Ernst, Reid Holmes, and Gordon Fraser. *Are Mutants a Valid Substitute for Real Faults in Software Testing?* FSE 2014. https://doi.org/10.1145/2635868.2635929
2. StrykerJS configuration: coverage analysis, per-test selection, concurrency, mutation scope, and timeouts. https://stryker-mutator.io/docs/stryker-js/configuration/
3. StrykerJS incremental mode and invalidation limitations. https://stryker-mutator.io/docs/stryker-js/incremental/
4. mutmut documentation, for comparison with a Python repository-oriented mutation workflow. https://mutmut.readthedocs.io/en/latest/
