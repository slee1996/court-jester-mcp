# Big Benchmark Runbook

Date: 2026-04-09

## Goal

Produce a benchmark package that is strong enough to support a release-readiness argument, not just an anecdotal demo.

The core question remains:

> Does Court Jester improve final task success in an agent loop without introducing enough false positives or infrastructure noise to make it net harmful?

## Why This Run Shape

A single matrix is not enough.

We need three separate evidence buckets:

1. utility on the real task pool
2. false-positive control on known-good code
3. library-spec pressure testing

We also need to reduce provider contamination. The CLI provider timeout for `claude-default` and `codex-default` is now `420s` instead of `240s` to reduce false infrastructure failures during the large run.

## Current Task-Set Sizes

As of this runbook:

- [core-current.json](../bench/task_sets/core-current.json): `39` tasks
- [library-slices.json](../bench/task_sets/library-slices.json): `2` tasks
- [known-good-corpus.json](../bench/task_sets/known-good-corpus.json): `8` local tasks
- [external-known-good-replay.json](../bench/task_sets/external-known-good-replay.json): `4` external gold-patch replay tasks
- [swebench-lite-known-good.json](../bench/task_sets/swebench-lite-known-good.json): `1` focused SWE-bench-style replay

The core set is already large enough to be meaningful.

The library set is still too small to carry the release case by itself. The known-good controls are now materially better than the original two-task sample, but they are still supporting evidence rather than a standalone release case.

## Recommended Run Package

### 1. Core Utility Run

```bash
python -m bench.run_matrix \
  --task-set core-current \
  --models claude-default,codex-default \
  --policies baseline,repair-loop \
  --repeats 3 \
  --output-dir /tmp/court-jester-core-big
```

Run count:

- `39 tasks x 2 models x 2 policies x 3 repeats = 468 runs`

This is the primary release-signal run.

### 2. Library Pressure Run

```bash
python -m bench.run_matrix \
  --task-set library-slices \
  --models claude-default,codex-default \
  --policies baseline,repair-loop \
  --repeats 5 \
  --output-dir /tmp/court-jester-library-big
```

Run count:

- `2 tasks x 2 models x 2 policies x 5 repeats = 40 runs`

This is a pressure test for spec-conformance repair behavior, not the main release argument.

### 3. Known-Good False-Positive Run

```bash
python -m bench.run_matrix \
  --task-set known-good-corpus \
  --models noop \
  --policies required-final \
  --repeats 10 \
  --output-dir /tmp/court-jester-known-good-big
```

Run count:

- `2 tasks x 1 model x 1 policy x 10 repeats = 20 runs`

This is the current false-positive control.

### 4. Summaries

```bash
python -m bench.summarize_runs /tmp/court-jester-core-big
python -m bench.summarize_runs /tmp/court-jester-library-big
python -m bench.summarize_runs /tmp/court-jester-known-good-big
```

## How To Read The Results

### Utility Gate

Success criteria:

- `repair-loop` beats `baseline` overall on the core run
- the lift is not entirely due to one task or one repeat
- at least one model below the saturation point shows meaningful lift

Warning signs:

- `repair-loop` is flat or worse overall
- Codex results are dominated by `provider_error`
- Claude regresses materially under `repair-loop`

### False-Positive Gate

Success criteria:

- zero hard verify failures on the known-good corpus
- no repair needed for known-good tasks

Warning signs:

- any `verify_stronger_than_eval` classification
- repeated stage failures on already-correct code

### Reliability Gate

Success criteria:

- low provider timeout rate relative to total runs
- run completion rate high enough to trust the matrix
- no obvious MCP instability or temp-file leakage

Warning signs:

- repeated `provider_error` outcomes at the CLI layer
- missing or partial result directories
- systematic timeout clusters by one model

## What This Run Can Prove

If this package is clean, it can support a claim like:

- Court Jester has credible evidence of utility on a broad adversarial fixture set
- Court Jester is not obviously harming already-correct code on the current control corpus
- Court Jester is operationally stable enough for a small private beta

It still does **not** prove:

- broad readiness for arbitrary external repos
- low false-positive rates across a large known-good corpus
- strong utility on upstream-derived library-spec tasks in general

## What Would Make The Evidence Stronger

Before using this as the primary release case, expand these two sets:

- library slices: grow from `2` to `6-10` tasks
- external known-good replay: grow from `4` to `8-12` tasks

The strongest next package would be:

- core utility: current `468` runs
- expanded library slices: roughly `160` runs
- expanded external known-good replay: roughly `80-120` runs

## Recommended Execution Order

1. Run known-good first.
This catches fresh false positives before spending time on the large utility matrix.

2. Run core utility second.
This is the main evidence run.

3. Run library slices third.
This adds pressure-test evidence without blocking the main release read.

4. Run the stress harness separately if the goal is a full private-beta packet.

## Notes

- The benchmark harness supports `--repeats`, and each repeat gets a paired hidden seed per task.
- The CLI model manifests now allow `420s` before classifying a run as a provider timeout.
- If Codex still produces too many provider timeouts after the timeout increase, treat that as an operational result in its own right rather than mixing it into the code-quality conclusion.
