# Court Jester Benchmark Report

Date: 2026-03-26

## Summary

We now have a clean enough benchmark run to make a real product claim:

Court Jester appears to improve final code quality for weaker models in an agent loop.

On the current 6-task slice:

- total runs: `36`
- total successes: `30`
- baseline: `14/18`
- repair-loop: `16/18`

The main lift comes from the weaker hosted model:

- `actual-api-qwen3-14b`
  - baseline: `2/6`
  - repair-loop: `4/6`
- `claude-default`
  - baseline: `6/6`
  - repair-loop: `6/6`
- `codex-default`
  - baseline: `6/6`
  - repair-loop: `6/6`

This does not prove Court Jester helps every model equally. It does show that it can materially improve outcomes for a weaker model on realistic cross-file tasks.

Final run artifacts:

- `bench/results/three-model-six-task-final`

## What We Benchmarked

We ran a 6-task slice across three models and two policies.

Models:

- `actual-api-qwen3-14b`
- `claude-default`
- `codex-default`

Policies:

- `baseline`
- `repair-loop`

Tasks:

- `py-semantic-profile-empty-name`
- `py-primary-plan-code-cross-file`
- `py-query-string-canonicalization`
- `ts-primary-title-missing`
- `ts-primary-plan-code-cross-file`
- `ts-semver-max-stable-cross-file`

The goal of this slice was not broad coverage. It was to answer one product question:

Does Court Jester help agents produce stronger final code than a one-shot baseline?

## Why We Trust This Run More Than Earlier Runs

Earlier benchmark iterations were contaminated by infrastructure and harness issues.

We fixed:

- MCP subprocess stdin inheritance that could collapse server sessions
- hidden-evaluator workspace path bugs
- Codex schema/timeout issues
- Actual API response parsing and transport flakiness
- weak repair prompts that allowed models to hand-wave away failing repros

The current run finished all `36` tasks without provider failures.

That matters because we are finally measuring model-and-loop behavior, not mostly harness noise.

## What Improved

The strongest current evidence is that Court Jester helps `actual-api-qwen3-14b`.

Concrete wins:

### 1. `py-primary-plan-code-cross-file`

- baseline: failed
- repair-loop: passed

What happened:

- the first patch only handled missing or empty plans
- Court Jester provided a concrete failing repro
- the model repaired the implementation successfully under the stronger counterexample-driven repair prompt

### 2. `ts-primary-plan-code-cross-file`

- baseline: failed
- repair-loop: passed

This is an important win because it is a TypeScript cross-file task rather than a trivial syntax problem.

In plain language, the loop did the thing we want:

1. the model produced a plausible but incomplete patch
2. Court Jester disproved it with a concrete failure
3. the model repaired it into a passing solution

That is the product behavior we are aiming for.

## What Did Not Improve

Court Jester did not materially improve the already-stronger frontier models on this slice.

That is not necessarily bad news.

It likely means one of two things:

1. those models are already saturating the current 6-task slice
2. this slice is too easy to show additional lift for them

The more important point is that the verifier did not degrade those models in the current run.

## Remaining Actual API Failures

The hosted weaker model still failed on two tasks.

### 1. `py-query-string-canonicalization`

Baseline:

- `hidden_semantic_miss`

Repair-loop:

- `verify_caught_public_bug`

Interpretation:

- baseline looked reasonable but still missed hidden semantics
- in repair-loop, the model changed the code, but Court Jester/public verification found a concrete bug before hidden evaluation
- this is an improvement in observability, but not yet a solved task

This is the current shape of a useful-but-insufficient loop:

1. the verifier successfully prevented a bad patch from being counted as success
2. the model still did not converge to the correct final implementation

### 2. `ts-semver-max-stable-cross-file`

Baseline:

- `public_check_failure`

Repair-loop:

- `verify_caught_public_bug`

Interpretation:

- the model is still weak on harder TypeScript cross-file semver reasoning
- it gets trapped in local fixes instead of coherent module-level reasoning
- Court Jester can catch the mistakes, but the model still struggles to repair them into a correct final patch

This is currently the clearest “model capability ceiling” in the slice.

## What We Changed To Improve Repair Behavior

The most important prompt change was making repair feedback more counterexample-driven.

We now tell the model:

- treat failing repros as authoritative
- your patch must change behavior on those repros
- do not claim the code is already correct if the cited repro still fails
- return `blocked` rather than pretending success if you cannot fix it

This mattered.

Before that change, weaker models would often:

- make a partial fix
- receive a concrete failing repro
- still claim the code was already correct

After the change, at least one of those failure modes converted into a real repair-loop win.

## What We Learned About Codex

One earlier Codex failure turned out not to be a model-quality miss.

It was a timeout artifact.

After restoring a more reasonable timeout for harder TypeScript cross-file tasks:

- `ts-semver-max-stable-cross-file`
  - baseline: passed
  - repair-loop: passed

That matters because it shows why benchmark hygiene is so important.

Without that fix, we would have been incorrectly counting a timing artifact as a code-quality failure.

## Current Product Read

For an average engineer evaluating whether this is worth using, the current answer is:

Yes, there is now credible evidence that Court Jester is useful.

But the evidence is specific:

1. it appears most useful for weaker models that still need help converting plausible code into correct code
2. it is less obviously useful on a small slice for already-strong frontier models
3. it is better at surfacing and breaking bad solutions than it is at guaranteeing that every model can repair them

That is still a meaningful product result.

“Helps weaker agents produce stronger final code” is enough to justify continued work.

## Recommended Next Steps

### 1. Expand the task slice

The current 6-task slice is good enough to show signal, but too small to generalize broadly.

Next step:

- expand to `20-50` tasks

### 2. Keep reporting infra-noise separately

We now distinguish model-quality failures from provider or infrastructure failures. That should remain part of every summary.

### 3. Focus on the hard remaining task families

The remaining misses suggest where the next work belongs:

- hidden semantic Python normalization/canonicalization tasks
- harder TypeScript cross-file semantic reasoning
- semver-style logic with helper/caller interaction

### 4. Keep repair-loop as the primary product path

`required-final` is a useful control.

`repair-loop` is the real product.

That is where Court Jester earns its keep.

## Bottom Line

As of March 26, 2026, the cleanest current benchmark result supports continuing the project.

Not because Court Jester is perfect.

Because it is now doing the important thing:

it is helping at least one weaker model produce stronger final code than baseline generation alone.
