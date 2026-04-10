# Repository Guidelines

## Project Structure & Module Organization
`src/` contains the Rust MCP server. `src/main.rs` wires stdio transport plus one-shot CLI subcommands (`verify`, `analyze`, `lint`, `execute`). `src/lib.rs` defines tool parameters and server routing. `src/tools/` holds the implementation for the four exposed MCP tools — `analyze`, `lint`, `execute`, `verify` — plus the internal `diff` and `synthesize` modules that `verify` composes. `tests/` contains Rust integration tests, usually grouped by tool (`verify_test.rs`, `diff_test.rs`, `synthesize_test.rs`). `scripts/smoke_mcp.py` is a tiny stdio MCP client used by `just smoke`. `bench/` is a separate Python benchmark harness with task manifests, evaluators, fixture repos, and runner scripts. `docs/` stores design notes and benchmark writeups.

## Build, Test, and Development Commands
The canonical commands live in the `justfile` — prefer those so the README, CI, and contributor docs stay in sync.

- `just build` (or `cargo build --release`): build the release binary.
- `just serve` (or `cargo run`): start the MCP server over stdio.
- `just smoke` / `just smoke-sample`: run the Python smoke client against the binary.
- `just verify-sample`: one-shot verify of the bundled fixture via the new CLI subcommand.
- `just test` (or `cargo test`): run the Rust integration test suite.
- `just fmt`: format Rust sources before review.
- `just bench-dry-run`: validate the Python benchmark matrix without running agents.
- `just bench-summarize <output-dir>`: summarize benchmark results.

Rust toolchain: **MSRV is 1.85** (`rmcp-macros` requires edition2024). Homebrew's stock `cargo` is 1.83 and will fail to build — install via `rustup` (`rustup default stable`) and make sure `~/.cargo/bin` comes before `/opt/homebrew/bin` on `PATH`. The `justfile` prepends `~/.cargo/bin` for you.

## Coding Style & Naming Conventions
Follow default Rust formatting via `cargo fmt`; use 4-space indentation and keep functions/modules in `snake_case`, types/traits in `PascalCase`, and constants in `SCREAMING_SNAKE_CASE`. Match the existing test style: concise helper builders plus descriptive test names such as `missing_preferred_timezone_fails_verify`. Python in `bench/` should stay simple and script-like, also using `snake_case`.

## Testing Guidelines
Add or update Rust tests in `tests/` for every behavior change. Prefer targeted cases that exercise parse, lint, execute, and verify stages directly. For benchmark changes, run at least `python -m bench.run_matrix --dry-run`; when modifying manifests or evaluators, include a real run summary if practical. Keep generated artifacts out of git; `target/`, `bench/results/`, and `__pycache__/` are already ignored.

## Commit & Pull Request Guidelines
This repository currently has no established commit history, so use short imperative subjects, for example `Add diff-aware verify filtering`. Keep commits focused. PRs should explain the behavioral change, list verification commands run, and call out any toolchain or benchmark impact. Include doc updates when behavior or workflow changes.
