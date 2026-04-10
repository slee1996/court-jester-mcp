# Court Jester Benchmark Update

Date: 2026-04-10

## Summary

We now have a stronger release-evidence package than the earlier six-task report.

The strongest clean utility result is the current `core-current` run on two frontier models:

- `claude-default`
  - baseline: `106 / 117`
  - repair-loop: `116 / 117`
- `codex-default`
  - baseline: `108 / 117`
  - repair-loop: `117 / 117`

That is a real utility signal:

- Claude improved by `+10` tasks
- Codex improved by `+9` tasks

The clean false-positive control also improved:

- known-good corpus under `noop + required-final`
  - `20 / 20` success

The current caveat is provider health, not benchmark logic.

Fresh Codex and Spark reruns on 2026-04-10 are currently hitting provider-side internal server errors. After adding fast retry and better classification, those runs now fail quickly as `provider_infra_error` instead of burning the full timeout budget.

So the current read is:

- Court Jester has real clean evidence of utility
- the false-positive control currently looks clean on the small known-good corpus
- the benchmark harness now reports provider failures much more honestly
- live Codex and Spark provider health is currently unstable, so fresh same-day reruns are infra-contaminated

## Clean Utility Evidence

This is the strongest clean run currently available for release-readiness evaluation:

- task set: `core-current`
- tasks: `39`
- models: `claude-default`, `codex-default`
- policies: `baseline`, `repair-loop`
- repeats: `3`

That produces `117` runs per model-policy pair.

### Claude

- baseline: `106 / 117`
- repair-loop: `116 / 117`

Failure shape:

- baseline: `11` hidden semantic misses
- repair-loop: `1` hidden semantic miss

Interpretation:

- Court Jester is not just catching bugs for Claude
- on this suite it is converting hidden misses into final task wins

### Codex

- baseline: `108 / 117`
- repair-loop: `117 / 117`

Failure shape on the clean run:

- baseline: `8` hidden semantic misses
- baseline: `1` provider failure
- repair-loop: `0` failures

Interpretation:

- the clean run still shows strong repair-loop lift
- Codex reached a perfect final score on that completed clean matrix

## False-Positive Control

The current known-good control is still small, but it is now clean:

- task set: `known-good-corpus`
- policy: `required-final`
- model: `noop`
- repeats: `10`
- result: `20 / 20`

That matters because an earlier run exposed a real false-positive path in TypeScript alias handling. After fixing synthesis, the same control corpus passes cleanly.

This is not enough to declare the false-positive problem solved in general, but it does clear the immediate known-good blocker on the current sample.

## Provider Reliability Update

On 2026-04-10, fresh Codex and Spark reruns stopped being useful as quality evidence because the provider started returning internal server errors broadly.

What changed in the harness:

- CLI providers now abort early on obvious provider-fatal output instead of waiting for the full timeout
- transient provider failures are retried with workspace rollback
- failure kinds are split into `usage_limited`, `capacity_busy`, `internal_server_error`, and `transport_error`

That means the harness now distinguishes:

- real code-quality failures
- verifier failures
- provider outages

### Codex rerun state

Fresh targeted reruns after the provider patch:

- library slices baseline rerun: `6 / 6` `provider_infra_error`
- single-task smoke rerun: `1 / 1` `provider_infra_error`

Common signature:

- `Transport channel closed`
- `UnexpectedContentType`
- `Internal server error`

Interpretation:

- this is not a library-slice-specific problem
- this is a broad Codex provider outage at the moment of rerun

### Spark rerun state

Fresh Spark rerun on library slices:

- first `2 / 2` completed runs: `provider_infra_error`
- run stopped early after the same provider signature appeared again

Interpretation:

- Spark is not currently usable as clean benchmark evidence either

## What We Can Claim Now

We can claim all of this honestly:

- Court Jester has clean benchmark evidence of improving final task success on a larger suite than the earlier six-task slice
- the current known-good control passes cleanly
- the harness now handles provider failures much better operationally
- benchmark summaries now separate provider outages from model-quality misses

We should not currently claim:

- that fresh Codex reruns are a clean measure of model quality today
- that Spark is reliable enough to be a headline benchmark model right now
- that false positives are fully characterized beyond the current known-good sample

## Bottom Line

The project case is stronger than it was in March.

The clean benchmark signal is now:

- real utility on a larger suite
- clean known-good control on the current sample

The remaining blocker is not “does Court Jester help?”

The remaining blocker is:

- how much more false-positive coverage we want before broader rollout
- whether the external provider environment is healthy enough to keep producing clean frontier-model reruns on demand
