# Court Jester 0.1.11

Date: 2026-04-22

## Summary

This release closes the main authoritative-test gap left in `0.1.10`.

Since `0.1.10`, Court Jester can now choose the TypeScript test runner explicitly, and the default `auto` path no longer forces Bun-native `bun:test` files through the Node runner.

## Highlights

- Authoritative TypeScript tests now have an explicit runner control.
  - added `--test-runner auto|node|bun|repo-native`
  - `verify` records both `test_runner_requested` and `test_runner_selected` in the `test` stage detail
- `auto` mode now handles Bun-native test files more realistically.
  - when the authoritative test imports `bun:test`, Court Jester prefers Bun for that stage even if the repo is not otherwise marked Bun-native
  - Node remains the default path for ordinary TypeScript authoritative tests
- Test coverage around runner selection is stronger and less flaky.
  - verifier integration tests now use project-local fake tools instead of mutating global `PATH`, which keeps the full suite stable under parallel `cargo test`

## Validation

Validated for this release:

- full Rust test suite with the pinned repo toolchain:
  - `/bin/zsh -lc 'RUSTC=$(rustup which rustc) RUSTDOC=$(rustup which rustdoc) rustup run stable cargo test -- --nocapture'`
- targeted regression coverage was added for:
  - `--test-runner` flag parsing
  - Bun selection for authoritative tests that import `bun:test`
  - authoritative test-stage detail reporting of requested vs selected runner
  - parallel-safe verifier tests using project-local tool resolution

## Known Limits

- `--test-runner auto` only switches to Bun when the authoritative test input clearly requires it, such as an import from `bun:test`; it is not a full general-purpose runtime inference layer.
- `repo-native` only resolves to Bun today. Other repo-native JavaScript runtimes are still out of scope.
- TypeScript lint still depends on a project-local, sibling, or `PATH` Biome binary.
