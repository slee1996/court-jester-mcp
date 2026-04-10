# Court Jester MCP — canonical commands.
#
# `just` is optional: every recipe is a short one-liner you can copy into a shell.
# This file just exists so the README, CI, and new contributors all read the same
# commands instead of drifting.

# The rmcp-macros dependency requires edition2024, so cargo must be >= 1.85.
# If your system cargo is Homebrew 1.83, rustup usually has a newer one —
# export this to force rustup's shims to take priority.
export PATH := env_var_or_default("HOME", "") + "/.cargo/bin:" + env_var_or_default("PATH", "")

# List all recipes.
default:
    @just --list

# Build the release binary at target/release/court-jester-mcp.
build:
    cargo build --release

# Build debug binary (faster compile, slower runtime).
build-debug:
    cargo build

# Run the Rust integration test suite.
test:
    cargo test

# Start the MCP server over stdio. Will appear to hang — that is normal; it is
# waiting for an MCP client to connect on stdin.
serve:
    cargo run --quiet

# End-to-end smoke test: initialize + tools/list over stdio.
smoke: build
    python3 scripts/smoke_mcp.py --release

# End-to-end smoke test including a real verify call against the bundled fixture.
smoke-sample: build
    python3 scripts/smoke_mcp.py --release --verify-sample

# One-shot verify of the bundled sample fixture via the new CLI subcommand.
verify-sample: build
    ./target/release/court-jester-mcp verify \
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
