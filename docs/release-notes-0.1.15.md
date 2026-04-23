# Court Jester 0.1.15

Date: 2026-04-23

## Summary

This release packages the current local verifier work into a single public cut. The theme is adoption: make reports easier to trust in day-to-day use, broaden the callable surface beyond top-level functions, and ship a first-party changed-files CI wrapper instead of asking every team to script one from scratch.

## Highlights

- Added `verify --summary human` for an interactive CLI summary.
- Split lint runner failures from ordinary lint findings so `lint_issues` stays honest.
- Added source-level `court-jester-ignore complexity` directives.
- Added explicit declarative execute checks with `court-jester-properties ...`.
- Surfaced exported object-literal methods and zero-arg exported class methods as real callable APIs.
- Made factory-returned methods explicit in coverage via `fuzzed_via_factory`.
- Added explicit support for Zustand-style container patterns such as `create(... => ({ ... }))` and curried `create<T>()(... )`.
- Added `court-jester ci --base ... --gate ... --report ...` for first-party PR workflows.

## Validation

Validated for this release:

- `/bin/zsh -lc 'RUSTC=$(rustup which rustc) RUSTDOC=$(rustup which rustdoc) rustup run stable cargo test -- --nocapture'`
- `/bin/zsh -lc 'RUSTC=$(rustup which rustc) rustup run 1.86.0-aarch64-apple-darwin cargo build --release --locked'`

## Known Limits

- Container support is still explicit and pattern-based. `0.1.15` supports surfaced callable discovery for Zustand-style `create(... => ({ ... }))` stores; it does not claim generic visibility into arbitrary hook bodies, RxJS chains, or framework internals.
- `court-jester ci` currently targets changed-file verification for Python and TypeScript source files. It reuses existing verify semantics and is intentionally narrower than a full monorepo workflow orchestrator.
