# Court Jester Benchmark Methodology

This document explains how the Court Jester benchmark is designed, what each suite is trying to prove, how the harness runs each cell, and how to read the results without making sloppy claims.

It is intentionally a methodology writeup, not a single-run results page.

## Goal

The benchmark is trying to answer one product question:

> Does Court Jester improve final task success in an agent loop without introducing enough false positives or operational noise to make it net harmful?

That splits into two separate requirements:

1. utility
2. precision

Utility means Court Jester helps the model end in a better final state than it would have without the verifier.

Precision means Court Jester does not block already-correct code too often.

Any benchmark that only measures one of those is incomplete.

## Design Principles

The harness is built around a few hard rules.

### 1. Separate the verifier from the benchmark orchestrator

`court-jester` itself is the Rust CLI that analyzes, executes, lints, and verifies code.

The Python harness under [`bench/`](../bench/) is responsible for:

- copying fixture repos into fresh workspaces
- running model/provider adapters
- applying policy logic
- calling `court-jester verify`
- running public and hidden evaluators
- recording artifacts and summaries

This separation matters because it keeps the benchmark logic from being baked into the product binary.

### 2. Do not treat every suite as the same kind of evidence

A known-good control is not the same thing as a utility benchmark.

A mutation-recall lane is not the same thing as a final-task-success benchmark.

A fresh framework clone task is not the same thing as a small semantic repair task.

The harness therefore uses named task sets with explicit suite roles instead of pretending everything belongs in one giant aggregate.

### 3. Use causal comparison policies, not just "with Court Jester" vs "without Court Jester"

The benchmark is not only comparing:

- baseline
- repair loop

It also supports policy variants that isolate different mechanisms:

- `baseline`
- `repair-loop-verify-only`
- `public-repair-*`
- `retry-*-no-verify`

That lets us ask tighter questions:

- did `verify` itself trigger the extra attempt?
- would public tests alone have done the same thing?
- is the gain just extra search budget?

### 4. Keep hidden evaluation final-score only

Hidden checks exist to score the final outcome, not to act like an oracle inside the repair loop.

If hidden failures are fed back into the model prompt, the benchmark stops measuring a realistic product loop and starts measuring an evaluator-assisted repair system.

## Benchmark Units

The smallest benchmark unit is a cell:

- one task
- one model
- one policy
- one repeat

For example:

- `ts-query-string-canonicalization`
- `claude-default`
- `repair-loop-verify-only`
- repeat `2/3`

Each cell gets its own copied workspace and its own result directory.

## Task Anatomy

Each task manifest in [`bench/tasks/`](../bench/tasks/) describes:

- a fixture repo to copy
- the prompt shown to the model
- which files are expected to change
- which files Court Jester should verify
- public checks
- hidden checks
- optional setup commands
- optional gold patch metadata

The important task categories in the current harness are:

- curated semantic repair tasks
- local known-good controls
- external gold-patch replay controls
- mutation seeds
- library/spec slices
- framework-clone slices

### Curated semantic repair tasks

These are the core product-utility tasks. They are small enough to run repeatedly, but adversarial enough to expose hidden semantic misses.

The main suite is [`bench/task_sets/core-current.json`](../bench/task_sets/core-current.json).

### Local known-good controls

These are already-correct implementations shipped as tasks. They are used to detect false positives on local fixture code.

The main suite is [`bench/task_sets/known-good-corpus.json`](../bench/task_sets/known-good-corpus.json).

### External gold-patch replay controls

These start from buggy upstream-derived tasks, but instead of asking a provider to edit code, the harness applies a task-local gold patch and then runs verify, public checks, and hidden checks normally.

This is the strongest current precision control because it asks:

> Does Court Jester wrongly block the known-good fix?

The main suite is [`bench/task_sets/external-known-good-replay.json`](../bench/task_sets/external-known-good-replay.json).

### Mutation seeds

These start from known-good code, then copy a real buggy source file from an existing fixture into the workspace during setup.

This lane is for verifier recall, not end-to-end solve rate.

The main suite is [`bench/task_sets/verify-mutation-seeds-v1.json`](../bench/task_sets/verify-mutation-seeds-v1.json).

## Policy Anatomy

Policies in [`bench/policies/`](../bench/policies/) define what can trigger another attempt and what feedback the model sees before that next attempt.

This is the most important attribution mechanism in the harness.

### `baseline`

- one attempt
- no Court Jester call
- no repair loop

This answers:

> What happens if we just let the model edit once and score the result?

### `repair-loop-verify-only`

- Court Jester runs after each attempt
- only a failed `verify` can trigger a repair attempt
- public or hidden failures do not drive the next prompt

This is the current headline policy because it isolates verifier-driven repair.

### `public-repair-*`

- no Court Jester call
- a failed public check can trigger a repair attempt
- hidden checks remain final-score only

This compares verify-guided repair against plain public-test-driven repair.

### `retry-*-no-verify`

- no Court Jester call
- no evaluator feedback between attempts
- extra attempts are spent blindly

This is the search-budget control. It asks whether the gain comes from the verifier or simply from giving the model more shots.

For the full control-loop diagrams, see [`bench/loop-diagrams.md`](../bench/loop-diagrams.md).

## Run Lifecycle

For each cell, the harness does the following:

1. Copy the fixture repo into a fresh workspace.
2. Apply any task `setup_commands`.
3. If the run is in gold-patch replay mode, apply `gold_patch_path` instead of calling a provider.
4. Otherwise call the provider adapter with the task prompt and policy-specific instructions.
5. Run `court-jester verify` if the policy requires it.
6. Run public checks.
7. Run hidden checks for scoring.
8. Decide whether another attempt is allowed and what feedback, if any, the model sees.
9. Write `run.json`, `result.json`, diffs, and optional trace artifacts.

That separation is important:

- `verify` is a runtime bug detector and repair trigger
- public checks are visible tests
- hidden checks are evaluator-only scorekeepers

## Suite Roles

The benchmark is deliberately split into suite kinds. The ones that matter most right now are:

### `headline_curated`

Main suite: `core-current`

Purpose:

- headline utility number
- repeated semantic repair benchmark

This is the suite to use when making the product claim:

> Court Jester helps or does not help overall on the current adversarial task pool.

### `false_positive_control`

Main suite: `known-good-corpus`

Purpose:

- local already-correct control

This is the first guardrail before trusting a utility win.

### `external_false_positive_control`

Main suite: `external-known-good-replay`

Purpose:

- broader upstream-derived precision control using known-good patches

This is stronger than local known-good because it measures whether Court Jester blocks the real fix for tasks grounded in upstream behavior.

### `verify_mutation_recall`

Main suite: `verify-mutation-seeds-v1`

Purpose:

- bug-detection recall
- seeded-bug failure-kind coverage

This lane should not be reported as if it were an end-to-end solve benchmark.

### `library_spec_slice`

Main suite: `library-slices`

Purpose:

- bounded pressure testing on library/spec semantics

This is supporting evidence, not the main release case.

### `framework_clone_*`

Main suites:

- `express-clone-alpha-pilot`
- `express-clone-alpha-monolith`
- `express-clone-alpha-fresh-*`

Purpose:

- repo-shaped framework evaluation
- shared-library and subsystem construction pressure

These are closer to the long-term product thesis, but they are harder to keep clean and should not be mixed casually with the smaller semantic task suites.

## Controls And Guardrails

The methodology only works if the benchmark has honest controls.

### Precision controls

The main precision controls are:

- `known-good-corpus` under `noop + required-final`
- `external-known-good-replay` under `noop + required-final --use-task-gold-patches`

These answer:

- does Court Jester wrongly fail already-correct local code?
- does Court Jester wrongly block the known-good upstream fix?

### Recall controls

The main recall control is:

- `verify-mutation-seeds-v1` under `noop + advisory`

This answers:

- when we inject seeded real bugs into known-good code, does verify catch them?

### Search-budget controls

The main search-budget controls are:

- `retry-once-no-verify`
- `retry-twice-no-verify`

These answer:

- is the gain really coming from the verifier, or just from giving the model more attempts?

### Public-test controls

The main mechanism comparison is:

- `public-repair-*` vs `repair-loop-verify-only*`

This answers:

- is Court Jester doing anything beyond what visible tests alone would do?

## Repeats, Randomization, And Scheduling

The harness supports repeated runs per cell because single-shot results are too noisy.

### Repeats

Each repeat creates another independent cell for the same task/model/policy.

This helps measure:

- stability
- repair conversion consistency
- whether a win is real or just a one-off

### Hidden seeds

Each repeat gets a paired hidden seed for the task. That makes repeat structure meaningful instead of purely duplicate work.

### `blocked-random` scheduling

The harness defaults to `--schedule blocked-random`.

That randomizes block order while preserving cell pairing structure. The goal is to reduce drift from long serial provider runs without fully scrambling the matrix.

### Provider isolation

The harness can run serially or with `--parallel-by-provider`. The latter keeps one serial queue per provider while still allowing different providers to progress concurrently.

That reduces wall-clock without turning the benchmark into an unbounded provider stress test.

## Scoring And Failure Categories

Each final cell lands in a structured outcome category.

The most important categories are:

- `success`
- `hidden_semantic_miss`
- `public_check_failure`
- `verify_caught_hidden_bug`
- `verify_caught_public_bug`
- `verify_stronger_than_eval`
- `provider_error`

### Success

The candidate passed:

- the policy's verify gate, if required
- public checks
- hidden checks

### Hidden semantic miss

The code looked plausible enough to survive visible checks but still failed the hidden evaluator.

These are the misses Court Jester is meant to reduce.

### Verify-caught bug

Court Jester failed the patch before the final hidden score would have counted it as a success. This is usually good, but only if the verifier is right and not overreaching.

### `verify_stronger_than_eval`

This is the key false-positive red flag:

- verify failed
- but public and hidden evaluators passed

If this shows up in a known-good lane, the precision story is not clean.

### Provider error

Provider failures should be tracked separately from code-quality outcomes whenever possible. They are operational data, not semantic evidence.

## Metrics We Care About

The harness records many fields. The most important ones are:

### Utility metrics

- `successes`
- `success_rate`
- `additional_successes_vs_baseline`
- `repair_trigger_sources`
- `repaired_after_verify_failure`
- `verify_recovery_rate`

These answer:

- did the loop save tasks?
- what actually triggered the extra attempts?
- when verify failed, how often did the loop recover?

### Precision metrics

- known-good pass rate
- replay success on gold-patch lanes
- `verify_false_positives`
- `verify_stronger_than_eval`

These answer:

- is Court Jester wrongly blocking good code?

### Recall metrics

- `verify_recall`
- `verify_true_positives`
- `verify_false_negatives`
- `verify_failure_kind_hit_rate`

These matter on mutation lanes, not on end-to-end utility lanes.

### Efficiency metrics

- `avg_attempts`
- `avg_end_to_end_ms`
- `successes_per_hour`
- `product_successes_per_hour`

These matter, but they are secondary to correctness. We do not treat a faster wrong system as a win.

## How To Read Each Lane Correctly

This is where benchmark writeups often go wrong.

### `core-current`

Read this by:

- final task success
- lift over baseline
- failure-category mix
- repair-trigger attribution

Do not use it as a precision control.

### `known-good-corpus`

Read this by:

- whether already-correct code passes cleanly

Do not use it as proof of recall.

### `external-known-good-replay`

Read this by:

- replay success

Do not over-read `verify_expectation_metrics` here. The underlying task manifests still encode the buggy-state expectation, so that table is intentionally misleading for gold-patch replay.

### `verify-mutation-seeds-v1`

Read this by:

- recall
- false negatives
- failure-kind coverage

Do not report it as if it were a final-task-success benchmark.

## Recommended Release-Evidence Package

The current recommended package is:

1. `known-good-corpus` with `noop + required-final`, `repeats=10`
2. `external-known-good-replay` with `noop + required-final --use-task-gold-patches`, `repeats=10`
3. `core-current` with `claude-default,codex-default` and `baseline,repair-loop-verify-only`, `repeats=3`
4. `library-slices` with the same models and policies, `repeats=5`

That structure gives:

- precision first
- then utility
- then spec-pressure support

See [`docs/big-benchmark-runbook.md`](big-benchmark-runbook.md) for the exact run package and pass criteria.

## Threats To Validity

The benchmark is stronger than it used to be, but it still has real failure modes.

### 1. Hidden-test contamination

If hidden or verifier-only test files are copied into the workspace, the benchmark is invalid because the agent can read them.

The harness avoids this by materializing hidden and verify-only assets outside the provider workspace and injecting them only during evaluation where needed.

### 2. Provider instability

Timeouts, auth failures, and CLI-provider drift can look like benchmark regressions if they are not classified separately.

That is why provider errors should not be collapsed into semantic misses by default.

### 3. Fixture overfitting

Micro-fixture suites are useful, but they can overstate generality. That is why the methodology keeps separate external replay, mutation, library, and framework lanes.

### 4. Path-sensitive replay artifacts

Gold-patch replay can behave differently if output workspaces are created in the wrong place. The harness needs replay runs to be semantically grounded, not accidentally dependent on path layout.

### 5. Search-budget confounding

If the benchmark only compares baseline to a repair loop, a lift could be caused by any second chance, not by the verifier specifically.

That is why blind-retry and public-repair comparison policies exist.

## Current Limits

The methodology is strong enough to support careful product claims, but it still does not prove:

- broad readiness on arbitrary external repos
- a globally low false-positive rate outside the current control corpus
- complete separation between verifier value and additional-attempt value until the multishot ablations are run more broadly

It is a disciplined benchmark, not a proof of universal performance.

## Practical Bottom Line

The benchmark approach is designed to support claims in this order:

1. Court Jester does or does not help on the main curated utility suite.
2. That help does or does not survive broader false-positive controls.
3. That help does or does not seem to come from verify-guided repair rather than from public tests or blind extra attempts.

If a result does not survive those three questions, it is not strong enough to headline.
