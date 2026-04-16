# Court Jester 0.1.5

Date: 2026-04-16

## Summary

This release hardens the verifier and benchmark harness for private-beta use and removes a real TypeScript sandbox false positive that was blocking a clean release cut.

## Highlights

- TypeScript sandbox execution now prefers Node's built-in `--experimental-transform-types` path instead of relying on `tsx` as the primary runner.
  - This fixes sandbox `EPERM` IPC failures that were causing false positives in execute/verify.
  - Safe TypeScript code, cross-file imports, top-level `await`, and `import.meta` now run under a Node-compatible path in the sandbox.
- `verify --tests-only` is no longer allowed to silently pass without an authoritative test.
  - Tests-only mode now requires a test file/code and fails explicitly otherwise.
  - Benchmark tasks that use tests-only verify now attach the authoritative test to every verified path instead of only the first one.
- Provider-process cleanup is more reliable.
  - Fast-path CLI children are tracked for interrupt cleanup.
  - Session-wide teardown is in place so benchmark interrupts do not leave orphaned CLI workers behind.
- Claude benchmark reliability is improved.
  - Idle-timeout handling no longer assumes streaming output from final-JSON CLI mode.
  - Bare auth mode is explicit opt-in instead of toggling on API-key presence alone.
  - Agent-trace environment injection is per-subprocess instead of process-global, which removes cross-thread leakage in parallel benchmarks.
- The repo is pinned to Rust `1.86.0` via [`rust-toolchain.toml`](../rust-toolchain.toml).

## Validation

Validated for this release:

- full Rust test suite under Rust `1.86.0`
- strict `cargo clippy --all-targets -- -D warnings` under Rust `1.86.0`
- Python benchmark harness unit tests:
  - `python -m unittest discover -s bench -p 'test_*.py'`
- benchmark dry-run:
  - `python -m bench.run_matrix --dry-run --task-set express-clone-alpha-public-repair-stress-v1 --models codex-default,claude-default --policies baseline,public-repair-1,repair-loop-verify-only,public-repair-2,repair-loop-verify-only-2 --parallel-by-provider --output-dir /tmp/court-jester-release-dry-run`
- release smoke:
  - `python3 scripts/smoke_cli.py --release --verify-sample`

## Known Limits

- The benchmark harness is operationally healthier, but the current public-repair stress suite still does not fully force tests-only multishot repair to engage on real model runs.
- The private-beta release case is stronger than before, but the benchmark-design work is still ongoing.
