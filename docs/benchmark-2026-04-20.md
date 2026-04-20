# Court Jester Causal Benchmark Package

Date: 2026-04-20

This is the strongest benchmark package in the repo so far.

It combines four things in one story:

- clean false-positive controls
- a matched one-repair causal matrix on `core-current`
- a public-repair proving ground
- a matched two-repair robustness matrix on `core-current`

The narrow claim supported by this package is:

- verifier-guided repair beats one-shot baseline
- verifier-guided repair beats public-test-guided repair
- verifier-guided repair beats blind extra retries
- the gain survives a larger retry budget
- the verifier is not buying that gain through obvious false positives on the current control lanes

## Precision Controls

Authoritative control artifacts:

- [2026-04-18-paper-known-good-r10-v2](/Users/spencerlee/court-jester-mcp/bench/results/matrix/2026-04-18-paper-known-good-r10-v2)
- [/tmp/cj-paper-external-known-good-r10](/tmp/cj-paper-external-known-good-r10)

Results:

- local known-good: `80 / 80`
- external upstream replay: `190 / 190`
- combined false-positive gauntlet: `270 / 270`

This is the precision floor for the rest of the package. If the verifier were simply becoming more aggressive, this control lane should have deteriorated first. It did not.

## One-Repair Primary Causal Matrix

Artifact:

- [2026-04-18-paper-core-causal-r3-v2](/Users/spencerlee/court-jester-mcp/bench/results/matrix/2026-04-18-paper-core-causal-r3-v2)

Run:

- task set: `core-current`
- tasks: `39`
- models: `claude-default`, `codex-default`
- policies: `baseline`, `public-repair-1`, `retry-once-no-verify`, `repair-loop-verify-only`
- repeats: `3`

Final result:

| Policy | Result | Success rate |
| --- | --- | --- |
| `baseline` | `208 / 234` | `88.9%` |
| `public-repair-1` | `205 / 234` | `87.6%` |
| `retry-once-no-verify` | `216 / 234` | `92.3%` |
| `repair-loop-verify-only` | `230 / 234` | `98.3%` |

By model:

| Model | Baseline | Public repair | Blind retry | Verify-only |
| --- | --- | --- | --- | --- |
| `claude-default` | `101 / 117` | `98 / 117` | `108 / 117` | `115 / 117` |
| `codex-default` | `107 / 117` | `107 / 117` | `108 / 117` | `115 / 117` |

Lift:

- verify-only vs baseline: `+22` successes, `+9.4` points
- verify-only vs blind retry: `+14` successes, `+6.0` points
- verify-only vs public repair: `+25` successes, `+10.7` points

Interpretation:

- verifier-guided repair beat both matched controls
- public-test-guided repair did not explain the gain
- a generic extra attempt did help, but it did not catch verifier-guided repair

## One-Repair Public-Repair Proving Ground

Artifact:

- [2026-04-19-paper-proving-ground-r3](/Users/spencerlee/court-jester-mcp/bench/results/matrix/2026-04-19-paper-proving-ground-r3)

Run:

- task set: `public-repair-proving-ground`
- tasks: `6`
- models: `claude-default`, `codex-default`
- policies: `baseline`, `public-repair-1`, `retry-once-no-verify`, `repair-loop-verify-only`
- repeats: `3`

Final result:

| Policy | Result | Success rate |
| --- | --- | --- |
| `baseline` | `11 / 36` | `30.6%` |
| `public-repair-1` | `14 / 36` | `38.9%` |
| `retry-once-no-verify` | `19 / 36` | `52.8%` |
| `repair-loop-verify-only` | `25 / 36` | `69.4%` |

By model:

| Model | Baseline | Public repair | Blind retry | Verify-only |
| --- | --- | --- | --- | --- |
| `claude-default` | `2 / 18` | `6 / 18` | `9 / 18` | `9 / 18` |
| `codex-default` | `9 / 18` | `8 / 18` | `10 / 18` | `16 / 18` |

Interpretation:

- public repair did help on the suite designed to favor it, so it was a fair live comparator
- even there, verifier-guided repair still won clearly
- the mechanism story is strongest on Codex, but the aggregate ranking still favors verifier-guided repair

## Two-Repair Robustness Matrix

Artifact:

- [2026-04-19-paper-core-robustness-r2](/Users/spencerlee/court-jester-mcp/bench/results/matrix/2026-04-19-paper-core-robustness-r2)

Run:

- task set: `core-current`
- tasks: `39`
- models: `claude-default`, `codex-default`
- policies: `baseline`, `public-repair-2`, `retry-twice-no-verify`, `repair-loop-verify-only-2`
- repeats: `2`

Stable final result:

| Policy | Result | Success rate |
| --- | --- | --- |
| `baseline` | `137 / 156` | `87.8%` |
| `public-repair-2` | `140 / 156` | `89.7%` |
| `retry-twice-no-verify` | `150 / 156` | `96.2%` |
| `repair-loop-verify-only-2` | `156 / 156` | `100.0%` |

By model:

| Model | Baseline | Public repair 2 | Blind retry 2 | Verify-only 2 |
| --- | --- | --- | --- | --- |
| `claude-default` | `67 / 78` | `66 / 78` | `75 / 78` | `78 / 78` |
| `codex-default` | `70 / 78` | `74 / 78` | `75 / 78` | `78 / 78` |

Lift:

- verify-only-2 vs baseline: `+19` successes, `+12.2` points
- verify-only-2 vs blind retry 2: `+6` successes, `+3.8` points
- verify-only-2 vs public repair 2: `+16` successes, `+10.3` points

Interpretation:

- giving everyone more budget helped both public repair and blind retry
- verifier-guided repair still finished best
- public repair still underperformed baseline on Claude
- the verifier-guided result did not collapse once the controls got more search budget

## Mechanism Read

Across the main causal package, the useful ingredient is not simply "another try."

The evidence package now says:

1. one extra blind retry helps, but not as much as verifier-guided repair
2. public-test-guided repair is a fair live comparator, but it is still weaker
3. the effect survives when everyone gets two repair chances
4. the false-positive controls remain clean alongside those runs

That is the strongest causal story currently supported by the repo.

## What This Package Supports

Supported:

- Court Jester improves final task success on the current repeated semantic repair benchmark
- the gain is not explained by public-test-guided repair
- the gain is not explained by blind extra retries alone
- the verifier does not show an obvious precision collapse on the current control lanes

Still not supported by this package alone:

- arbitrary-repo readiness
- broad external validity beyond the current curated and replayed suites
- cost- or latency-dominance as a standalone claim
- universal superiority over every possible test-driven repair workflow

## Practical Read

The April 18 package showed that verifier-guided repair still helped after false-positive tightening.

This April 20 package is stronger because it adds the missing controls:

- matched public repair
- matched blind retry
- a public-repair proving ground
- a two-step robustness rerun

That turns the repo story from:

- "verify-only helps on this benchmark"

into:

- "verifier-guided repair beats both public-test-guided repair and blind retries on this benchmark package, while remaining clean on the current false-positive gauntlet"

## Superseded Writeups

- [benchmark-2026-04-18.md](benchmark-2026-04-18.md) remains useful as the first broader false-positive-plus-utility package
- this document is now the strongest finished benchmark summary in the repo
