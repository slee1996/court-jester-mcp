# Court Jester Benchmark Update

Date: 2026-04-10

## Summary

We now have a stronger causal benchmark than the earlier repeated `repair-loop` run.

The strongest current release-evidence result is a strict verify-only repair policy on the `core-current` task set:

- task set: `core-current`
- tasks: `39`
- models: `claude-default`, `codex-default`
- policies: `baseline`, `repair-loop-verify-only`
- repeats: `1`
- total runs: `156`

Headline result:

- `claude-default`
  - baseline: `35 / 39`
  - verify-only repair loop: `37 / 39`
- `codex-default`
  - baseline: `36 / 39`
  - verify-only repair loop: `39 / 39`

Aggregate:

- baseline: `71 / 78`
- verify-only repair loop: `76 / 78`

This is the important part:

- `11` runs triggered a repair round because `court-jester verify` failed
- all `11` trigger sources were exactly `verify`
- `0` public-trigger repair rounds
- `0` hidden-trigger repair rounds
- `10` verify-triggered repair rounds ended in final success

So the current clean claim is:

- Court Jester improves final task success on this suite
- the improvement is not coming from public or hidden evaluator feedback being fed back into the model
- the current benchmark still does not separate verify feedback from the value of getting any second attempt at all

That last point matters. This benchmark is strong enough to show that Court Jester is responsible for the repair trigger and the repair feedback. It is not yet a blind-retry ablation.

## What We Ran

Command:

```bash
python -m bench.run_matrix \
  --task-set core-current \
  --models codex-default,claude-default \
  --policies baseline,repair-loop-verify-only \
  --repeats 1 \
  --output-dir /tmp/court-jester-core-cli-verify-only-rerun
```

Summary command:

```bash
python -m bench.summarize_runs /tmp/court-jester-core-cli-verify-only-rerun
```

## Why This Run Is Stronger

The earlier large run showed utility, but it still left a real attribution question:

- was the lift coming from Court Jester itself?
- or was it partly coming from later public or hidden evaluator feedback leaking into the repair prompt?

This run closes that gap.

The `repair-loop-verify-only` policy is:

- one repair round maximum
- only triggered after a failed `verify`
- public and hidden failures do not trigger repair feedback

That behavior is encoded directly in:

- [bench/policies/repair-loop-verify-only.json](../bench/policies/repair-loop-verify-only.json)
- [bench/runner.py](../bench/runner.py)

The run output confirms the policy actually behaved that way:

- Claude repair trigger sources: `{"verify": 6}`
- Codex repair trigger sources: `{"verify": 5}`
- no public-trigger repairs
- no hidden-trigger repairs

So the utility signal is no longer confounded by evaluator feedback.

## Utility Read

### Claude

- baseline: `35 / 39`
- verify-only repair loop: `37 / 39`

Failure shape:

- baseline: `4` hidden semantic misses
- verify-only: `1` hidden semantic miss
- verify-only: `1` provider error

Interpretation:

- Court Jester still improves Claude on this suite under the stricter policy
- one remaining failure is still a real semantic miss
- one remaining failure is provider-side, not code-quality evidence

### Codex

- baseline: `36 / 39`
- verify-only repair loop: `39 / 39`

Failure shape:

- baseline: `3` hidden semantic misses
- verify-only: `0` failures

Interpretation:

- Codex reached a clean `39 / 39` on this run
- the stricter policy did not erase the earlier utility signal

## Repair Conversion Read

Inside the repair-loop arm itself:

- Claude:
  - `6` repair attempts
  - `5` verify-triggered repairs ended in success
- Codex:
  - `5` repair attempts
  - `5` verify-triggered repairs ended in success

This matters because the overall policy lift is only `+5` tasks relative to baseline, but the internal loop behavior is stronger than that aggregate delta suggests. The repair loop frequently rescues its own first-attempt failures, even when baseline on a separately sampled run happened to pass.

That is the right product read:

- Court Jester is doing useful work inside the loop
- the aggregate benchmark delta is a conservative headline, not the whole story

## Remaining Failures

Remaining non-successes in the strict run:

1. `py-query-string-canonicalization` on `claude-default`
- failure category: `hidden_semantic_miss`
- Court Jester still found enough to trigger a verify-based repair, but Claude did not converge to the final correct patch

2. `ts-semver-caret-prerelease` on `claude-default`
- failure category: `provider_error`
- this should not be treated as evidence about Court Jester quality

## False-Positive Control

The known-good control remains:

- task set: `known-good-corpus`
- policy: `required-final`
- model: `noop`
- repeats: `10`
- result: `20 / 20`

That is still a small sample, but it keeps the immediate false-positive blocker cleared.

## What We Can Claim Now

We can claim all of this honestly:

- Court Jester improves final task success on the current 39-task suite
- the strongest current lift comes from a strict verify-only repair policy
- that lift is not explained by public or hidden evaluator feedback being fed back into the model
- the known-good control still passes on the current sample
- provider failures are now classified separately enough that they do not get mistaken for code-quality misses

We should not yet claim:

- that verify feedback beats a blind second chance
- that the false-positive story is fully characterized beyond the small known-good corpus
- that provider health is stable enough to make every same-day frontier-model rerun clean

## Bottom Line

The release case is stronger than it was this morning.

The current best evidence is no longer just “repair-loop beats baseline.” It is:

- verify-only repair-loop beats baseline
- all repair feedback in that run came from Court Jester verify
- the tool is still catching and converting real model mistakes into final wins

The next benchmark step should be a blind-retry ablation, not another round of cleanup on this same question.
