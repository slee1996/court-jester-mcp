# Benchmark Loop Diagrams

This document diagrams the benchmark control loops that matter for current Court Jester attribution work.

The goal is to make three things obvious:

1. What causes another attempt to happen.
2. What feedback, if any, the model sees before that next attempt.
3. Which checks are only for final scoring vs. which checks can actually trigger repair.

## Terms

- `attempt`: one provider call that edits the workspace
- `repair round`: one extra attempt beyond the first
- `max attempts`: `1 + max_repair_rounds`
- `public checks`: visible tests shipped in the fixture repo
- `hidden checks`: evaluator-only tests used for scoring
- `verify`: `court-jester verify`

## At A Glance

| Policy | Max attempts | Calls `verify`? | What triggers another attempt? | What feedback is shown to the model? | When do hidden checks matter? |
| --- | --- | --- | --- | --- | --- |
| `baseline` | 1 | No | Never | None | Final scoring only |
| `public-repair-1` | 2 | No | Failed public checks | Public test failure output | Final scoring only |
| `public-repair-2` | 3 | No | Failed public checks | Public test failure output | Final scoring only |
| `repair-loop-verify-only` | 2 | Yes | Failed `verify` | Verify repros only | Scoring only; hidden failure does not trigger repair |
| `repair-loop-verify-only-2` | 3 | Yes | Failed `verify` | Verify repros only | Scoring only; hidden failure does not trigger repair |
| `retry-once-no-verify` | 2 | No | Nothing; always spends the extra attempt budget | None | Only the final attempt is judged |
| `retry-twice-no-verify` | 3 | No | Nothing; always spends the full extra attempt budget | None | Only the final attempt is judged |

## Operator Notes

- `repair-loop-verify-only*` still runs public checks after each attempt, and may run hidden checks during repair attempts for telemetry and scoring. Those results do not drive the next prompt. Only `verify` can do that.
- `public-repair-*` never feeds hidden failures back to the model. Hidden checks are final-score only in those policies.
- `retry-*-no-verify` is a pure search-budget control. Non-final attempts are intentionally not judged.
- In `verify`-only policies, `verify` has trigger priority. If `verify` fails and public/hidden also fail on the same attempt, the next prompt still gets only verify-driven feedback.

## 1. Baseline

```text
attempt 1
  -> provider edits code
  -> public checks
  -> hidden checks
  -> final score
```

Properties:

- No Court Jester calls
- No repair loop
- One-shot benchmark lane

## 2. Public Repair x1

Policy: `public-repair-1`

```text
attempt 1
  -> provider edits code
  -> public checks
       -> pass: hidden checks -> final score
       -> fail: feed public failure output -> attempt 2

attempt 2
  -> provider edits code
  -> public checks
  -> hidden checks
  -> final score
```

Properties:

- No Court Jester calls
- Public checks can trigger one repair round
- Hidden checks never trigger repair

## 3. Public Repair x2

Policy: `public-repair-2`

```text
attempt 1
  -> provider edits code
  -> public checks
       -> pass: hidden checks -> final score
       -> fail: feed public failure output -> attempt 2

attempt 2
  -> provider edits code
  -> public checks
       -> pass: hidden checks -> final score
       -> fail: feed public failure output -> attempt 3

attempt 3
  -> provider edits code
  -> public checks
  -> hidden checks
  -> final score
```

Properties:

- Same trigger as `public-repair-1`
- Two repair rounds, three total attempts
- Hidden is still final-score only

## 4. Verify-Only Repair x1

Policy: `repair-loop-verify-only`

```text
attempt 1
  -> provider edits code
  -> court-jester verify
  -> public checks
  -> hidden checks may run for scoring/telemetry
       -> verify failed: feed verify repros only -> attempt 2
       -> verify passed: stop repairing -> final score uses public + hidden + verify gate

attempt 2
  -> provider edits code
  -> court-jester verify
  -> public checks
  -> hidden checks
  -> final score
```

Properties:

- Only `verify` can trigger repair
- Public and hidden failures do not generate repair feedback
- Final success still requires public pass, hidden pass, and verify gate pass

## 5. Verify-Only Repair x2

Policy: `repair-loop-verify-only-2`

```text
attempt 1
  -> provider edits code
  -> court-jester verify
  -> public checks
  -> hidden checks may run
       -> verify failed: feed verify repros only -> attempt 2
       -> verify passed: stop repairing -> final score

attempt 2
  -> provider edits code
  -> court-jester verify
  -> public checks
  -> hidden checks may run
       -> verify failed: feed verify repros only -> attempt 3
       -> verify passed: stop repairing -> final score

attempt 3
  -> provider edits code
  -> court-jester verify
  -> public checks
  -> hidden checks
  -> final score
```

Properties:

- Same trigger semantics as `repair-loop-verify-only`
- Two repair rounds, three total attempts
- This is the direct multishot verify-guided comparison against `public-repair-2`

## 6. Blind Retry x1, No Verify

Policy: `retry-once-no-verify`

```text
attempt 1
  -> provider edits code
  -> no verify
  -> no public checks
  -> no hidden checks
  -> go straight to attempt 2

attempt 2
  -> provider edits code
  -> public checks
  -> hidden checks
  -> final score
```

Properties:

- Pure extra search budget
- No verifier
- No evaluator feedback between attempts
- Only the final attempt is judged

## 7. Blind Retry x2, No Verify

Policy: `retry-twice-no-verify`

```text
attempt 1
  -> provider edits code
  -> no verify
  -> no public checks
  -> no hidden checks
  -> go straight to attempt 2

attempt 2
  -> provider edits code
  -> no verify
  -> no public checks
  -> no hidden checks
  -> go straight to attempt 3

attempt 3
  -> provider edits code
  -> public checks
  -> hidden checks
  -> final score
```

Properties:

- Pure three-shot search-budget control
- Useful for testing whether more attempts alone can explain a lift
- Not a realistic product loop

## Current Comparison Sets

### Public Tests vs Verify

```text
baseline
public-repair-1
repair-loop-verify-only
public-repair-2
repair-loop-verify-only-2
```

This is the best comparison when the question is:

`Does Court Jester beat a normal public-test repair loop?`

### Blind Search Budget vs Verify

```text
baseline
retry-once-no-verify
repair-loop-verify-only
retry-twice-no-verify
repair-loop-verify-only-2
```

This is the best comparison when the question is:

`Does Court Jester beat simply giving the model more shots?`

## Practical Interpretation

When reading benchmark results:

- If `public-repair-*` does not fire any repair attempts, the corpus is not stressing public-test-driven repair enough.
- If `repair-loop-verify-only*` fires repairs and improves success, that is clean evidence that Court Jester is doing useful runtime work.
- If `retry-*-no-verify` improves much less than verify-guided repair, that is evidence the benefit is not just extra attempt budget.
