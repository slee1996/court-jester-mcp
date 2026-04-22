# Court Jester 0.1.13

Date: 2026-04-22

## Summary

This is a verifier behavior patch for Bun-backed authoritative TypeScript tests.

`0.1.11` added `--test-runner auto|node|bun|repo-native`, and `0.1.12` could correctly select Bun for files that import `bun:test`. The remaining bug was that the selected Bun path still executed the file in Bun script mode. `0.1.13` fixes that by invoking Bun's test runner directly for authoritative test stages.

## Highlights

- Bun-backed authoritative test stages now run as `bun test <file>`.
- `--test-runner bun` and `--test-runner auto` both execute `bun:test` files under Bun's test runner once Bun is selected.
- Added a regression test that asserts the Bun subcommand is `test`, not plain script execution.

## Validation

Validated for this release:

- `/bin/zsh -lc 'RUSTC=$(rustup which rustc) RUSTDOC=$(rustup which rustdoc) rustup run stable cargo test -- --nocapture'`
- `/bin/zsh -lc 'RUSTC=$(rustup which rustc) rustup run 1.86.0-aarch64-apple-darwin cargo build --release --locked'`

## Known Limits

- `--test-runner auto` only switches to Bun when the authoritative test input clearly requires Bun semantics, such as an import from `bun:test`.
- This release does not change the non-test TypeScript execute path.
