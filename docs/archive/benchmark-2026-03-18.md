# Court Jester Benchmark Report

Date: 2026-03-18

## Goal

Determine whether `court-jester verify` is useful in an agentic coding loop, and whether it should continue as an active line of work.

## Benchmark Setup

- Harness: `bench/run_matrix.py`
- Summary tool: `bench/summarize_runs.py`
- Tasks: 9 local Python/TypeScript fixture tasks
- Models:
  - `claude-default`
  - `codex-default`
- Policies:
  - `baseline`
  - `required-final`
  - `repair-loop`

Final full-matrix artifacts:

- `/tmp/court-jester-bench-full-v7`

## Current Result

Final rerun after semantic and runtime fixes:

- `54 / 54` runs succeeded
- Claude: `baseline 9/9`, `required-final 9/9`, `repair-loop 9/9`
- Codex: `baseline 9/9`, `required-final 9/9`, `repair-loop 9/9`

This replaces the earlier unstable result where `required-final` and `repair-loop` could fail on TypeScript `execute` timeouts even when public and hidden checks passed.

## What Changed

### 1. Lint warnings became informational

`verify` no longer treats TypeScript/Python lint diagnostics as hard failures unless the lint runner itself errors.

Why it mattered:

- removed a false positive on `ts-display-initials-anonymous`
- made `required-final` viable again for correct patches

Relevant files:

- `src/tools/verify.rs`
- `tests/verify_test.rs`

### 2. TypeScript semantic coverage improved

Added and fixed fuzz coverage for blank-string semantic bugs:

- label helpers returning `""`
- city-formatting helpers returning `""` after `.trim()`
- nested nullable type aliases like `type User = { ... } | null`
- inline object member generation such as `{ city?: string | null }`
- top-level union splitting that respects nested braces

Why it mattered:

- fixed the `ts-secondary-label-missing` false negative
- fixed the `ts-null-primary-city` false negative

Relevant files:

- `src/tools/analyze.rs`
- `src/tools/synthesize.rs`
- `tests/verify_test.rs`
- `tests/synthesize_test.rs`

### 3. TypeScript runtime stability improved

The largest remaining false-failure source was not semantics. It was TypeScript execution startup.

Fixes:

- reverted to a Node-first execution path instead of preferring Bun
- included inherited `PATH` in the sandbox so existing Node tooling is discoverable
- increased TypeScript `execute` timeout in `verify` from `10s` to `25s`

Why it mattered:

- removed `Process timed out` false failures in `required-final`
- removed `Process timed out` false failures in `repair-loop`
- made the city task pass cleanly under gated policies

Relevant files:

- `src/tools/sandbox.rs`
- `src/tools/verify.rs`

## Interpretation

Current evidence supports continuing work on `court-jester`.

What the benchmark now shows:

- `verify` is capable of catching real semantic bugs
- `verify` can be used inside a repair loop without obviously degrading outcomes on this suite
- `required-final` is no longer failing for infrastructure reasons on the tested tasks

What this does **not** prove:

- that the current 9-task suite is enough to generalize broadly
- that larger repos or slower TypeScript setups will never reintroduce timeout issues
- that the current heuristics have low false-positive/false-negative rates outside these fixtures

## Recommendation

Continue, but with the benchmark kept in the loop.

Sprint follow-up:

- `./sprint-2026-03-18.md`
- `./benchmark-and-fuzzing-2026-03-20.md`

Recommended next work:

1. Expand the task suite from 9 to 20-50 tasks.
2. Add more TypeScript tasks with slower startup/import paths.
3. Keep `required-final` and `repair-loop` in every rerun.
4. Track not just success rate, but when `verify` materially changes the final patch.

## Reproduction

Run the same matrix:

```bash
python -u -m bench.run_matrix \
  --models codex-default,claude-default \
  --policies baseline,required-final,repair-loop \
  --output-dir /tmp/court-jester-bench-full-v7
python -m bench.summarize_runs /tmp/court-jester-bench-full-v7
```

Current Codex benchmark default:

- `bench/models/codex-default.json` uses `gpt-5.1-codex-max`
