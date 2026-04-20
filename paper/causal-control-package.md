# Causal Control Package

## Goal

Turn the current benchmark story from:

- strong utility signal
- strong precision signal

into:

- strong utility signal
- strong precision signal
- strong causal attribution

The exact question is:

> Does verifier-guided repair beat both public-test-guided repair and blind extra retries on the same repeated benchmark package, without collapsing on false positives?

This document gives the exact run package to answer that.

## What We Need To Show

For the paper to survive the obvious skeptical read, we need all three of these:

1. `repair-loop-verify-only` beats `retry-*-no-verify`
2. `repair-loop-verify-only` beats `public-repair-*`
3. false-positive controls stay clean alongside those runs

Without that, a reviewer can still say:

- maybe any extra attempt would have helped
- maybe public tests alone would have done the same thing

## Execution Principles

- Keep provider matrices serial, not overlapping.
- Use `--schedule blocked-random`.
- Fix `--shuffle-seed 7` for all causal comparisons.
- Keep the false-positive controls in the same package, not as a historical footnote.
- Treat the one-repair package as primary evidence.
- Treat the two-repair package as robustness evidence.

## Package Overview

### Precision controls

1. `known-good-corpus`
2. `external-known-good-replay`

### Primary causal package

3. `core-current` one-repair matched-attempt matrix — completed at `bench/results/matrix/2026-04-18-paper-core-causal-r3-v2`
4. `public-repair-proving-ground` one-repair mechanism matrix — completed at `bench/results/matrix/2026-04-19-paper-proving-ground-r3`

### Robustness package

5. `core-current` two-repair matched-attempt matrix — completed at `bench/results/matrix/2026-04-19-paper-core-robustness-r2`
6. optional `public-repair-proving-ground` two-repair mechanism matrix — not yet run

### Optional supporting pressure

7. `library-slices` one-repair causal matrix

## Exact Commands

### 0. Dry-run sanity

```bash
python3 -m bench.run_matrix \
  --task-set core-current \
  --models claude-default,codex-default \
  --policies baseline,public-repair-1,retry-once-no-verify,repair-loop-verify-only \
  --repeats 1 \
  --schedule blocked-random \
  --shuffle-seed 7 \
  --dry-run
```

### 1. Local precision control

```bash
python3 -m bench.run_matrix \
  --task-set known-good-corpus \
  --models noop \
  --policies required-final \
  --repeats 10 \
  --schedule blocked-random \
  --output-dir bench/results/matrix/2026-04-18-paper-known-good-r10
```

Run count:

- `8 tasks x 1 model x 1 policy x 10 repeats = 80`

### 2. External replay precision control

```bash
python3 -m bench.run_matrix \
  --task-set external-known-good-replay \
  --models noop \
  --policies required-final \
  --use-task-gold-patches \
  --repeats 10 \
  --schedule blocked-random \
  --output-dir /tmp/cj-paper-external-known-good-r10
```

Run count:

- `19 tasks x 1 model x 1 policy x 10 repeats = 190`

### 3. Primary one-repair causal matrix on `core-current`

```bash
python3 -m bench.run_matrix \
  --task-set core-current \
  --models claude-default,codex-default \
  --policies baseline,public-repair-1,retry-once-no-verify,repair-loop-verify-only \
  --repeats 3 \
  --schedule blocked-random \
  --shuffle-seed 7 \
  --output-dir bench/results/matrix/2026-04-18-paper-core-causal-r3
```

Run count:

- `39 tasks x 2 models x 4 policies x 3 repeats = 936`

Why this matrix matters:

- `baseline` is the one-shot floor
- `retry-once-no-verify` matches extra attempt budget with no verifier
- `public-repair-1` matches extra attempt budget with visible-test feedback
- `repair-loop-verify-only` is the verifier-guided condition

This single matrix answers the main causal question at one extra attempt.

Observed result from `bench/results/matrix/2026-04-18-paper-core-causal-r3-v2`:
- baseline: `208 / 234` = `88.9%`
- `public-repair-1`: `205 / 234` = `87.6%`
- `retry-once-no-verify`: `216 / 234` = `92.3%`
- `repair-loop-verify-only`: `230 / 234` = `98.3%`
- Claude: `101 / 117` -> `115 / 117` under verify-only; public repair `98 / 117`; blind retry `108 / 117`
- Codex: `107 / 117` -> `115 / 117` under verify-only; public repair `107 / 117`; blind retry `108 / 117`

Interpretation:
- verify-only beat blind retry
- verify-only beat public repair
- public repair slightly underperformed baseline overall

### 4. One-repair public-repair mechanism matrix

```bash
python3 -m bench.run_matrix \
  --task-set public-repair-proving-ground \
  --models claude-default,codex-default \
  --policies baseline,public-repair-1,retry-once-no-verify,repair-loop-verify-only \
  --repeats 3 \
  --schedule blocked-random \
  --shuffle-seed 7 \
  --output-dir bench/results/matrix/2026-04-18-paper-proving-ground-r3
```

Run count:

- `6 tasks x 2 models x 4 policies x 3 repeats = 144`

Why this matrix matters:

- `core-current` is the headline suite, but public-repair may not always fire often enough there
- this smaller suite is designed so public-test repair has a fair chance to show what it can do

Without this matrix, a reviewer can still argue that the public-repair comparator was underpowered by task choice.

Observed result from `bench/results/matrix/2026-04-19-paper-proving-ground-r3`:
- baseline: `11 / 36` = `30.6%`
- `public-repair-1`: `14 / 36` = `38.9%`
- `retry-once-no-verify`: `19 / 36` = `52.8%`
- `repair-loop-verify-only`: `25 / 36` = `69.4%`
- Claude: baseline `2 / 18`, public repair `6 / 18`, blind retry `9 / 18`, verify-only `9 / 18`
- Codex: baseline `9 / 18`, public repair `8 / 18`; by subtraction from the aggregate totals, blind retry `10 / 18` and verify-only `16 / 18`

Interpretation:
- public repair does help on the suite designed to favor it, so it was a fair live comparator
- even on that suite, verify-only still wins clearly
- Claude is the messier model here; the mechanism signal is strongest on Codex

### 5. Two-repair robustness matrix on `core-current`

```bash
python3 -m bench.run_matrix \
  --task-set core-current \
  --models claude-default,codex-default \
  --policies baseline,public-repair-2,retry-twice-no-verify,repair-loop-verify-only-2 \
  --repeats 2 \
  --schedule blocked-random \
  --shuffle-seed 7 \
  --output-dir bench/results/matrix/2026-04-19-paper-core-robustness-r2
```

Run count:

- `39 tasks x 2 models x 4 policies x 2 repeats = 624`

Why only `repeats=2` here:

- this is a robustness package, not the primary headline
- it keeps cost reasonable while still testing whether the main conclusion survives a larger attempt budget

Final status from `bench/results/matrix/2026-04-19-paper-core-robustness-r2`:
- full matrix complete: `624` runs, `583` succeeded
- aggregate rows in the partial live view briefly mixed completion counts during final flush, so the stable read should come from the completed policy summaries rather than the transient `159/157` artifact
- stable aggregate totals from the completed per-model summaries:
  - baseline: `137 / 156` = `87.8%`
  - `public-repair-2`: `140 / 156` = `89.7%`
  - `retry-twice-no-verify`: `150 / 156` = `96.2%`
  - `repair-loop-verify-only-2`: `156 / 156` = `100.0%`
- stable per-model completed summaries:
  - Claude: baseline `67 / 78`, `public-repair-2` `66 / 78`, `retry-twice-no-verify` `75 / 78`, `repair-loop-verify-only-2` `78 / 78`
  - Codex: baseline `70 / 78`, `public-repair-2` `74 / 78`, `retry-twice-no-verify` `75 / 78`, `repair-loop-verify-only-2` `78 / 78`

Interpretation:
- more budget helped both public repair and blind retry
- verifier-guided repair still finished best
- public repair still underperformed baseline on Claude
- blind retry got stronger, but still did not catch verifier-guided repair

This completes the main two-step robustness package and keeps the causal ranking intact.

### 6. Two-repair public-repair mechanism matrix

```bash
python3 -m bench.run_matrix \
  --task-set public-repair-proving-ground \
  --models claude-default,codex-default \
  --policies baseline,public-repair-2,retry-twice-no-verify,repair-loop-verify-only-2 \
  --repeats 3 \
  --schedule blocked-random \
  --shuffle-seed 7 \
  --output-dir bench/results/matrix/2026-04-18-paper-proving-ground-r3-twostep
```

Run count:

- `6 tasks x 2 models x 4 policies x 3 repeats = 144`

### 7. Optional supporting library/spec matrix

```bash
python3 -m bench.run_matrix \
  --task-set library-slices \
  --models claude-default,codex-default \
  --policies baseline,public-repair-1,retry-once-no-verify,repair-loop-verify-only \
  --repeats 5 \
  --schedule blocked-random \
  --shuffle-seed 7 \
  --output-dir bench/results/matrix/2026-04-18-paper-library-causal-r5
```

Run count:

- `2 tasks x 2 models x 4 policies x 5 repeats = 80`

This is not necessary for the main causal claim, but it is useful supporting evidence because Court Jester is currently strongest on library/spec-style semantics.

## Summaries

Run after each matrix:

```bash
python3 -m bench.summarize_runs bench/results/matrix/2026-04-18-paper-known-good-r10
python3 -m bench.summarize_runs /tmp/cj-paper-external-known-good-r10
python3 -m bench.summarize_runs bench/results/matrix/2026-04-18-paper-core-causal-r3
python3 -m bench.summarize_runs bench/results/matrix/2026-04-18-paper-proving-ground-r3
python3 -m bench.summarize_runs bench/results/matrix/2026-04-18-paper-core-causal-r2-twostep
python3 -m bench.summarize_runs bench/results/matrix/2026-04-18-paper-proving-ground-r3-twostep
python3 -m bench.summarize_runs bench/results/matrix/2026-04-18-paper-library-causal-r5
```

## What To Report

For each primary matrix, report:

- successes
- success rate
- lift vs baseline
- lift vs `public-repair-*`
- lift vs `retry-*-no-verify`
- `verify_triggered_repairs`
- `verify_recovery_rate`
- failure-category mix

For the precision controls, report:

- `known-good-corpus`: pass rate
- `external-known-good-replay`: replay success rate

Do not report the replay lane using `verify_expectation_metrics`. Report it as replay success.

## Decision Rule

The paper claim survives if all of these are true.

### Precision

- `known-good-corpus` stays effectively clean
- `external-known-good-replay` stays effectively clean
- no new `verify_stronger_than_eval` headline blockers appear

### Primary causal result

On `core-current` one-repair matrix:

- `repair-loop-verify-only` beats `retry-once-no-verify` overall
- `repair-loop-verify-only` beats `public-repair-1` overall
- at least one model shows a clear positive gap over both controls

### Mechanism sanity

On `public-repair-proving-ground`:

- `public-repair-1` actually fires and rescues some tasks
- `repair-loop-verify-only` still matches or beats it

This is important because it prevents the unfair reviewer objection that the public-repair baseline was dead on arrival.

### Robustness

On the two-repair package:

- `repair-loop-verify-only-2` does not lose the story against `retry-twice-no-verify`
- `repair-loop-verify-only-2` does not lose the story against `public-repair-2`

This can be weaker than the one-repair result and still be useful. It just cannot reverse it.

## What Counts As Failure

The paper is still not ready if any of these happen:

- `retry-once-no-verify` ties or beats `repair-loop-verify-only` on the primary matrix
- `public-repair-1` ties or beats `repair-loop-verify-only` on the primary matrix
- public-repair never meaningfully fires in the proving-ground suite
- precision controls regress

If that happens, the paper story becomes:

- "extra attempts helped"

instead of:

- "verifier-guided repair helped"

## Minimal Submission-Ready Slice

If budget gets tight, the minimum package I would still trust is:

1. local known-good control
2. external replay control
3. `core-current` one-repair causal matrix
4. `public-repair-proving-ground` one-repair mechanism matrix

Everything else is strengthening, not minimum viability.

## Bottom Line

If the one-repair causal matrix survives against both public-repair and blind-retry controls while the precision gauntlet stays clean, then the paper becomes much harder to dismiss.

That is the exact next benchmark package to run.
