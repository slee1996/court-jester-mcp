# Repository Guidelines

## Project Structure & Module Organization
`src/` contains the Rust MCP server. `src/main.rs` wires the stdio transport, `src/lib.rs` defines tool parameters and server routing, and `src/tools/` holds the implementation for `analyze`, `lint`, `execute`, `verify`, `diff`, and `synthesize`. `tests/` contains Rust integration tests, usually grouped by tool (`verify_test.rs`, `diff_test.rs`, `synthesize_test.rs`). `bench/` is a separate Python benchmark harness with task manifests, evaluators, fixture repos, and runner scripts. `docs/` stores design notes and benchmark writeups.

## Build, Test, and Development Commands
Use Cargo for the Rust server:

- `cargo run`: start the MCP server over stdio.
- `cargo fmt`: format Rust sources before review.
- `cargo test`: run the Rust test suite in `tests/`.

Use Python for the benchmark harness:

- `python -m bench.run_matrix --dry-run`: validate the benchmark matrix without running agents.
- `python -m bench.summarize_runs <output-dir>`: summarize benchmark results.

Note: in this environment, `cargo test` currently fails with Cargo `1.83.0` because `rmcp-macros` requires `edition2024`; use a newer Rust toolchain before relying on test results.

## Coding Style & Naming Conventions
Follow default Rust formatting via `cargo fmt`; use 4-space indentation and keep functions/modules in `snake_case`, types/traits in `PascalCase`, and constants in `SCREAMING_SNAKE_CASE`. Match the existing test style: concise helper builders plus descriptive test names such as `missing_preferred_timezone_fails_verify`. Python in `bench/` should stay simple and script-like, also using `snake_case`.

## Testing Guidelines
Add or update Rust tests in `tests/` for every behavior change. Prefer targeted cases that exercise parse, lint, execute, and verify stages directly. For benchmark changes, run at least `python -m bench.run_matrix --dry-run`; when modifying manifests or evaluators, include a real run summary if practical. Keep generated artifacts out of git; `target/`, `bench/results/`, and `__pycache__/` are already ignored.

## Commit & Pull Request Guidelines
This repository currently has no established commit history, so use short imperative subjects, for example `Add diff-aware verify filtering`. Keep commits focused. PRs should explain the behavioral change, list verification commands run, and call out any toolchain or benchmark impact. Include doc updates when behavior or workflow changes.
