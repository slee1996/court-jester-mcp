# Court Jester Private Beta Release Checklist

Date: 2026-04-10

## Goal

Define a release bar for expanding Court Jester beyond a single-user workflow.

The standard is not:

- "the verifier can break Claude"
- "the verifier catches some bugs"

The standard is:

- Court Jester helps users ship stronger final code
- Court Jester does not regularly block correct code
- Court Jester is operationally reliable enough for repeated in-loop use

## Product Question

The headline release question should remain:

> Does Court Jester improve final task success in an agent loop without introducing enough false positives or instability to make it net harmful?

That implies three separate release gates:

1. Utility
2. False-positive control
3. Operational reliability

## Release Gates

### 1. Utility Gate

Required evidence:

- `repair-loop` beats `baseline` on a meaningful benchmark suite
- the suite is broad enough that the result is not a one-task anecdote
- the result holds on at least one model that is not already saturating the task set

Suggested minimum bar:

- `20-50` tasks
- Python and TypeScript coverage
- adversarial local fixtures
- upstream-derived library spec slices
- at least three model tiers

Metrics to report:

- overall `baseline` success rate
- overall `repair-loop` success rate
- success delta by model
- repair conversion rate after verify failure

### 2. False-Positive Gate

Required evidence:

- known-good implementations pass `verify`
- known-good implementations pass public and hidden checks
- `required-final` does not incorrectly block correct code at an unacceptable rate

This repo now has a dedicated false-positive control set:

- [known-good-corpus.json](../bench/task_sets/known-good-corpus.json)
- [external-known-good-replay.json](../bench/task_sets/external-known-good-replay.json)

Current initial tasks:

- [ts-lodash-array-slice-1-known-good.json](../bench/tasks/ts-lodash-array-slice-1-known-good.json)
- [ts-lodash-object-slice-1-known-good.json](../bench/tasks/ts-lodash-object-slice-1-known-good.json)

Suggested minimum bar:

- near-zero hard verify failures on the known-good corpus under `required-final`
- known-good tasks should not require repair to succeed

### 3. Reliability Gate

Required evidence:

- no CLI process collapse under stress
- no meaningful temp-file or sibling-file leakage
- acceptable p50 and p95 verify latency
- timeout rate low enough for normal agent-loop use

Metrics to report:

- verify timeout rate
- provider error rate separated from code-quality failures
- end-to-end benchmark completion rate
- p50/p95 verify duration by language

## Recommended Beta Positioning

Court Jester is ready for a private beta when it can be described honestly as:

- a hostile verifier for AI coding loops
- best for Python and TypeScript
- strongest on cross-file semantic bugs, hidden edge cases, nullish/fallback bugs, and repair-loop workflows
- not a general CI replacement
- not a secure hidden-judge system yet

## Minimum Evidence Package

Before expanding access beyond a single-user workflow, produce all of:

1. Utility benchmark summary
- `baseline` vs `repair-loop`
- by model
- by bug class

2. False-positive benchmark summary
- `required-final` results on the known-good corpus
- failures, if any, with concrete root-cause analysis

3. Reliability summary
- stress results
- timeout/error breakdown

4. User-facing setup docs
- install
- integration into an agent loop
- known limitations

5. Design-partner feedback
- at least `5-10` users
- whether they kept Court Jester enabled
- examples where it caught a real bug
- examples where it got in the way

## Suggested Commands

Utility benchmark:

```bash
python -m bench.run_matrix \
  --task-set core-current \
  --models codex-default,claude-default \
  --policies baseline,repair-loop-verify-only \
  --output-dir /tmp/court-jester-core-release
```

Library-slice benchmark:

```bash
python -m bench.run_matrix \
  --task-set library-slices \
  --models codex-default,claude-default \
  --policies baseline,repair-loop \
  --output-dir /tmp/court-jester-library-release
```

False-positive control benchmark:

```bash
python -m bench.run_matrix \
  --task-set known-good-corpus \
  --models noop \
  --policies required-final \
  --output-dir /tmp/court-jester-known-good
```

External replay false-positive benchmark:

```bash
python -m bench.run_matrix \
  --task-set external-known-good-replay \
  --models noop \
  --policies required-final \
  --use-task-gold-patches \
  --output-dir /tmp/court-jester-external-known-good
```

Then summarize:

```bash
python -m bench.summarize_runs /tmp/court-jester-core-release
python -m bench.summarize_runs /tmp/court-jester-library-release
python -m bench.summarize_runs /tmp/court-jester-known-good
python -m bench.summarize_runs /tmp/court-jester-external-known-good
```

## Current Read

Current evidence supports expanding beyond a single-user workflow.

Current evidence still does not justify a broad public claim that Court Jester is generally ready for all users, all repos, and all agent workflows.

Updated read on 2026-04-20:

- one-repair primary causal matrix on `core-current`:
  - `baseline`: `208 / 234`
  - `public-repair-1`: `205 / 234`
  - `retry-once-no-verify`: `216 / 234`
  - `repair-loop-verify-only`: `230 / 234`
- one-repair proving ground:
  - `baseline`: `11 / 36`
  - `public-repair-1`: `14 / 36`
  - `retry-once-no-verify`: `19 / 36`
  - `repair-loop-verify-only`: `25 / 36`
- two-repair robustness on `core-current`:
  - `baseline`: `137 / 156`
  - `public-repair-2`: `140 / 156`
  - `retry-twice-no-verify`: `150 / 156`
  - `repair-loop-verify-only-2`: `156 / 156`
- false-positive gauntlet:
  - `80 / 80` success on the local `known-good-corpus`
  - `190 / 190` success on `external-known-good-replay`
  - `270 / 270` combined clean control passes

Interpretation:

- the current package is stronger than the earlier April 18 utility rerun because it now includes matched public-repair and blind-retry controls
- verifier-guided repair beat both public-test-guided repair and blind extra retries in the primary matrix
- public repair remained a fair live comparator on the proving ground, but still lost there too
- the ranking survived a larger retry budget in the two-step robustness run
- the false-positive blocker remains cleared on both the local corpus and the broader upstream replay lane
- provider health and repo-shape limitations still matter, but the core utility and attribution story is now much harder to dismiss as "just retries helped"

Updated false-positive control result on 2026-04-17:

- initial run exposed a real blocker: [ts-lodash-object-slice-1-known-good.json](../bench/tasks/ts-lodash-object-slice-1-known-good.json) failed with `verify_stronger_than_eval`
- root cause: TypeScript fuzz synthesis treated unresolved named aliases such as `PathValue` as generic objects, producing impossible inputs for same-file helper functions
- after broadening the corpus and fixing export-surface detection plus malformed-URI rejection, the local [known-good-corpus.json](../bench/task_sets/known-good-corpus.json) passed `8/8` and then `16/16` over repeats under `noop + required-final`
- the broader external [external-known-good-replay.json](../bench/task_sets/external-known-good-replay.json) replay now covers `19` upstream-derived repo tasks across requests-style cookies, packaging, node-semver, lodash, qs, and fresh Express spec chunks, and it passed `19/19` and then `38/38` over repeats under `noop + required-final --use-task-gold-patches`

Two additional upstream replay probes were intentionally left out of the headline lane on 2026-04-17:

- `ts-express-fresh-router-core-v2` currently surfaces a real verifier false positive (`verify_stronger_than_eval`) on generic helper fuzz in `lib/http.ts`
- `ts-express-fresh-static-v3` still needs a better upstream-grounded gold source before it is an honest known-good replay item

That clears the immediate false-positive blocker on both the current local corpus and a broad external replay lane that is no longer dominated by a single library family. The next bar is pushing further into held-out repo-shaped controls and provider-backed false-positive validation.

The next milestone is clear:

- show utility on a broader benchmark
- show low false-positive rates on known-good tasks
- show stable operation under repeated use
