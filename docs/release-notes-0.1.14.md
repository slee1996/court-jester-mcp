# Court Jester 0.1.14

Date: 2026-04-22

## Summary

This is a fuzz-coverage patch for TypeScript collection types.

Before this release, functions with signatures like `Set<string>` could still be skipped as unsupported even when the collection contents were otherwise simple and fuzzable. `0.1.14` teaches the TypeScript synthesizer how to generate common collection generics directly so those functions stay in scope.

## Highlights

- Added fuzz-generator support for `Set<T>` and `ReadonlySet<T>`.
- Added fuzz-generator support for `Map<K, V>` and `ReadonlyMap<K, V>`.
- Added fuzz-generator support for `ReadonlyArray<T>`.
- Functions using collection generics like `Set<string>` are no longer skipped as unsupported.

## Validation

Validated for this release:

- `/bin/zsh -lc 'RUSTC=$(rustup which rustc) RUSTDOC=$(rustup which rustdoc) rustup run stable cargo test -- --nocapture'`
- `/bin/zsh -lc 'RUSTC=$(rustup which rustc) rustup run 1.86.0-aarch64-apple-darwin cargo build --release --locked'`

## Known Limits

- Seed-only fuzzing for still-unsupported TypeScript parameter types is still not implemented; seeds currently help only after a function is admitted into the fuzz plan.
- This release expands collection coverage, but it does not attempt full arbitrary generic-type synthesis.
