# Repository Guidelines

## Project Structure & Module Organization
`src/` contains the Rust CLI. `src/main.rs` is the agent-facing entrypoint for `verify`, `analyze`, `lint`, and `execute`. `src/lib.rs` exposes shared helpers such as language parsing, input resolution, and project-root detection. `src/tools/` holds the implementation for the four CLI commands plus the internal `diff` and `synthesize` modules that `verify` composes. `tests/` contains Rust integration tests, usually grouped by tool (`verify_test.rs`, `diff_test.rs`, `synthesize_test.rs`). `scripts/smoke_cli.py` is the end-to-end smoke test used by `just smoke`. `bench/` is a separate Python benchmark harness with task manifests, evaluators, fixture repos, CLI wrappers, and runner scripts. `docs/` stores design notes and benchmark writeups.

## Build, Test, and Development Commands
The canonical commands live in the `justfile` — prefer those so the README, CI, and contributor docs stay in sync.

- `just build` (or `cargo build --release`): build the release binary.
- `just smoke` / `just smoke-sample`: run the CLI smoke checks against the binary.
- `just verify-sample`: one-shot verify of the bundled fixture via the CLI.
- `just test` (or `cargo test`): run the Rust integration test suite.
- `just fmt`: format Rust sources before review.
- `just bench-dry-run`: validate the Python benchmark matrix without running agents.
- `just bench-summarize <output-dir>`: summarize benchmark results.

Rust toolchain: prefer the `rustup` toolchain when it differs from an older system Cargo. The `justfile` prepends `~/.cargo/bin` for you.

## Coding Style & Naming Conventions
Follow default Rust formatting via `cargo fmt`; use 4-space indentation and keep functions/modules in `snake_case`, types/traits in `PascalCase`, and constants in `SCREAMING_SNAKE_CASE`. Match the existing test style: concise helper builders plus descriptive test names such as `missing_preferred_timezone_fails_verify`. Python in `bench/` should stay simple and script-like, also using `snake_case`.

## Testing Guidelines
Add or update Rust tests in `tests/` for every behavior change. Prefer targeted cases that exercise parse, lint, execute, and verify stages directly. For benchmark changes, run at least `python -m bench.run_matrix --dry-run`; when modifying manifests or evaluators, include a real run summary if practical. Keep generated artifacts out of git; `target/`, `bench/results/`, and `__pycache__/` are already ignored.

## Commit & Pull Request Guidelines
This repository currently has no established commit history, so use short imperative subjects, for example `Add diff-aware verify filtering`. Keep commits focused. PRs should explain the behavioral change, list verification commands run, and call out any toolchain or benchmark impact. Include doc updates when behavior or workflow changes.
