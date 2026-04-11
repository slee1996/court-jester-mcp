# Court Jester System Flow

Date: 2026-03-19

## Goal

Describe the full Court Jester system as it exists today:

- agent loop
- verifier loop
- benchmark harness
- hidden evaluator path
- future external-judge direction

See also: [tool-flow-diagram.md](./tool-flow-diagram.md) for a single Mermaid view of the CLI commands and benchmark loop.

## High-Level Flow

```text
Task manifest
    |
    v
Benchmark runner
bench/run_matrix.py
bench/runner.py
    |
    +-------------------------------+
    |                               |
    v                               v
Model provider                   Public checks
bench/providers.py               visible tests
    |
    v
Patch candidate in temp workspace
    |
    v
Court Jester verify
bench/cli_client.py -> src/main.rs
    |
    +-------------------------------+
    |                               |
verify passes                  verify fails
    |                               |
    v                               v
Hidden evaluator               repair feedback
bench/evaluators/*             bench/runner.py
    |                               |
    |                               v
    |                         model repair attempt
    |                               |
    +---------------<---------------+
                    up to repair-loop limit
    |
    v
result.json + diff + artifacts
    |
    v
summary / bucket metrics
bench/summarize_runs.py
```

## Current Product Modes

### `baseline`

```text
model -> public checks -> hidden evaluator
```

### `required-final`

```text
model -> Court Jester verify gate -> public checks -> hidden evaluator
```

### `repair-loop`

```text
model -> verify fail -> repair -> verify -> public checks -> hidden evaluator
```

### `repair-loop-2`

```text
model -> verify fail -> repair -> verify fail -> repair -> verify -> public checks -> hidden evaluator
```

## Runner Flow

`bench/runner.py` is the orchestration layer.

For each `task x model x policy x repeat`:

1. Copy the repo fixture into a fresh temp workspace.
2. Snapshot the workspace before the model runs.
3. Run the model provider against the task prompt.
4. If the policy uses Court Jester, call `verify` on the configured `verify_paths`.
5. If `verify` fails and repair is allowed, build compact repro feedback and ask the model to repair.
6. Snapshot the workspace after the final attempt and save a diff.
7. Run public checks.
8. Run the hidden evaluator.
9. Write `result.json` with:
   - success/failure
   - failure category
   - verify results
   - repair attempts
   - changed files
   - command artifacts

Important current runner behavior:

- ignores cache noise in `.npm/`, `Library/`, `.ruff_cache/`, and `.DS_Store`
- normalizes `bytes` before JSON serialization
- generates a per-run hidden seed for hidden evaluators via `CJ_HIDDEN_SEED`

## Provider Flow

`bench/providers.py` currently supports:

- `codex_cli`
- `claude_cli`
- `noop`
- replay fixtures

Provider responsibility:

- apply code changes in the workspace
- return changed files, transcript, exit code, and parsed summary when available
- support repair feedback when the provider can take a second attempt

The benchmark does not assume providers are stable. Provider errors are explicitly classified and recorded.

## Court Jester Verify Flow

The benchmark shells out to the Rust CLI through `bench/cli_client.py`.

The `verify` command entrypoint is exposed from:

- `src/main.rs`
- `src/tools/verify.rs`

For each verified file, Court Jester runs a staged pipeline:

1. Parse / analyze
   - `src/tools/analyze.rs`
   - extracts functions, classes, imports, complexity

2. Synthesize fuzz harness
   - `src/tools/synthesize.rs`
   - generates language-aware adversarial inputs and property checks

3. Lint
   - informational unless the lint runner itself errors

4. Execute / fuzz
   - runs generated fuzz/property tests in the sandbox

5. File-based test stage
   - runs explicit public verify tests when configured

6. Report
   - JSON report per file with stage details and overall pass/fail

## Sandbox Flow

`src/tools/sandbox.rs` is responsible for running Python/TypeScript in a controlled subprocess.

Current important behavior:

- Node-first TypeScript execution path
- inherited `PATH` included so existing tooling is discoverable
- file/test-stage handling supports sibling imports and separate test files

This is where several earlier false failures came from, especially on TypeScript startup and cross-file test execution.

## Hidden Evaluator Flow

Hidden evaluators are the final semantic judge in the benchmark.

Current location:

- `bench/evaluators/*`

Two evaluator styles exist today:

### Fixed hidden evaluators

Older tasks use a fixed assertion list in Python or Bun.

### Seeded hidden evaluators

The new semver slice uses generated hidden cases derived from `CJ_HIDDEN_SEED`.

Current seeded examples:

- `bench/evaluators/ts_semver_compare_hidden.py`
- `bench/evaluators/ts_semver_caret_hidden.py`
- `bench/evaluators/ts_semver_max_hidden.py`

This is not true secrecy, but it is closer to the external-judge shape:

- exact hidden cases are generated at run time
- assertions are not all hardcoded as a literal visible list
- repair can still be driven by a compact repro rather than the full hidden suite

## Result Classification Flow

`bench/runner.py` assigns a `failure_category` after public checks, hidden checks, and verify outcome are known.

Important categories:

- `success`
- `hidden_semantic_miss`
- `verify_caught_hidden_bug`
- `verify_false_positive`
- `verify_infra_timeout`
- `provider_error`
- `provider_auth_error`

This is what lets the benchmark distinguish:

- Court Jester helping
- Court Jester hurting
- model failure without Court Jester involvement
- harness or provider instability

## What The System Is Good At Right Now

Current benchmark evidence supports Court Jester as an in-loop breaker:

- `repair-loop` and `repair-loop-2` are consistently strong
- false-positive clusters found by the benchmark have been fixable
- the current wide adversarial sweep shows `required-final` is still weaker than repair policies for Codex, mainly because it catches real hidden bugs without allowing recovery

The system is especially useful on:

- fallback-chain bugs
- nested nullable data bugs
- cross-file semantic bugs
- sparse-array and blank-string semantic bugs

## What The System Is Not Yet

Court Jester is not yet a true hidden-judge system.

Why:

- hidden evaluators are still local
- a determined model with shell/filesystem access can inspect them
- seeded hidden generation reduces exact visibility but does not create a hard secrecy boundary

That is acceptable for the current stage, because the immediate goal is product validation rather than benchmark hardening.

## Future External Judge Flow

The likely future shape is:

```text
model -> Court Jester verify -> repair loop -> external hidden evaluator -> result
```

That version would move hidden checks outside the model-visible workspace and return only:

- pass/fail
- minimized repro
- violated property

instead of exposing the evaluator implementation itself.

## Practical Summary

Today, the full system is:

- benchmark harness for orchestration
- model provider for code generation
- Court Jester verify for hostile pre-merge failure discovery
- hidden evaluator for semantic ground truth
- summary tooling for measuring whether Court Jester helps or hurts

The current product direction is clear:

- optimize `repair-loop`
- keep `required-final` as a control
- keep pushing on adversarial tasks and hidden evaluators
- delay external hidden-judge engineering until the utility signal is strong enough to justify it
