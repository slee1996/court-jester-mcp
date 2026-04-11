# Court Jester CLI — canonical commands.
#
# `just` is optional: every recipe is a short one-liner you can copy into a shell.
# This file just exists so the README, CI, and new contributors all read the same
# commands instead of drifting.

# Prefer rustup's cargo when it differs from an older system installation.
# This keeps contributor commands aligned with CI and the README.
export PATH := env_var_or_default("HOME", "") + "/.cargo/bin:" + env_var_or_default("PATH", "")

# List all recipes.
default:
    @just --list

# Build the release binary at target/release/court-jester.
build:
    cargo build --release

# Build debug binary (faster compile, slower runtime).
build-debug:
    cargo build

# Run the Rust integration test suite.
test:
    cargo test

# End-to-end smoke test: help/version plus optional verify call.
smoke: build
    python3 scripts/smoke_cli.py --release

# End-to-end smoke test including a real verify call against the bundled fixture.
smoke-sample: build
    python3 scripts/smoke_cli.py --release --verify-sample

# One-shot verify of the bundled sample fixture via the new CLI subcommand.
verify-sample: build
    ./target/release/court-jester verify \
        --file bench/repos/mini_py_service/profile.py \
        --language python \
        --project-dir bench/repos/mini_py_service

# Validate the Python benchmark matrix without running any agents.
bench-dry-run:
    python3 -m bench.run_matrix --dry-run

# Summarize benchmark results for a given output directory.
bench-summarize dir:
    python3 -m bench.summarize_runs {{dir}}

# Format Rust sources.
fmt:
    cargo fmt

# Clippy lint the Rust sources.
clippy:
    cargo clippy --all-targets -- -D warnings
