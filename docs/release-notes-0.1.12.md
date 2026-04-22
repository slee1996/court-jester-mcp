# Court Jester 0.1.12

Date: 2026-04-22

## Summary

This is a packaging-only patch release.

`0.1.11` updated the crate version without committing the matching `Cargo.lock` package entry, which caused release builds using `cargo build --release --locked` to fail. `0.1.12` ships that lockfile fix.

## Highlights

- Updated the root package entry in `Cargo.lock` to match the crate version.
- Restored successful `cargo build --release --locked` release builds.

## Validation

Validated for this release:

- `/bin/zsh -lc 'RUSTC=$(rustup which rustc) rustup run 1.86.0-aarch64-apple-darwin cargo build --release --locked'`

## Known Limits

- This release does not change verifier behavior beyond packaging/build correctness.
