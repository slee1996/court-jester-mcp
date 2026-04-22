# Court Jester 0.1.9

Date: 2026-04-22

## Summary

This release tightens the TypeScript verifier in the places where the last round of real-world runs was overstating confidence.

Since `0.1.8`, Court Jester now reports what it actually fuzzed, distinguishes portability failures from behavioral failures, and reaches more of the web-platform-shaped TypeScript code that previously got skipped.

## Highlights

- Fuzz coverage reporting is now explicit instead of implied.
  - `verify` emits per-function coverage statuses such as `fuzzed`, `skipped_unsupported_type`, `skipped_internal_helper`, and `blocked_module_load`.
  - report summaries now count skipped and module-load-blocked functions directly from the coverage stage.
- TypeScript execution is more honest and more useful on Bun-oriented repos.
  - Court Jester can fall back from strict Node ESM execution to the repo-native Bun runtime when the code clearly depends on Bun globals or Bun-style module resolution.
  - Node portability failures are still preserved as a separate `portability` signal instead of being silently ignored.
- The synthesizer reaches more of the functions that matter in security-heavy TypeScript code.
  - added generators for `Headers`, `Request`, `Response`, and `URLSearchParams`
  - non-exported top-level parser and normalizer helpers with simple inputs can now be fuzzed directly
- Intentional entropy helpers no longer create fake determinism failures.
  - zero-argument helpers with UUID, correlation-ID, timestamp, nonce, token, random, and similar naming patterns now skip the consistency property.
- The CLI has a simpler security gate.
  - `--profile security` maps to a complexity threshold of `20` unless an explicit threshold is provided.

## Product Changes Since 0.1.8

- `synthesize` now builds a structured fuzz plan instead of only emitting raw generated calls.
- `verify` now emits dedicated `coverage` and `portability` stages for TypeScript runs.
- The sandbox keeps TypeScript fuzz harnesses closer to the source file when package resolution matters, which preserves repo-local imports more reliably.
- Bun-oriented repos can now execute through the repo runtime when strict Node module loading would otherwise stop fuzzing at import time.

## Validation

Validated for this release:

- full Rust test suite with the pinned repo toolchain:
  - `/bin/zsh -lc 'RUSTC=$(rustup which rustc) RUSTDOC=$(rustup which rustdoc) /Users/spencerlee/.rustup/toolchains/1.86.0-aarch64-apple-darwin/bin/cargo test'`
- targeted regression coverage was added for:
  - entropy-helper determinism exemptions
  - internal helper fuzzing
  - web-platform TypeScript generators
  - Bun runtime fallback and repo-local import resolution
  - verify coverage and portability stage reporting

## Known Limits

- Signed request and webhook verification paths still benefit from hand-written fixtures; generic generation is better on parser-shaped inputs than on end-to-end authenticated payloads.
- Repo-native TypeScript fallback currently focuses on Bun-shaped repos. Other runtime-specific environments may still stop at module load.
- Complexity reporting is easier to operationalize in this release, but the tool still only reports hotspots; it does not simplify them for you.
