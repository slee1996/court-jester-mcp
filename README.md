# Court Jester MCP

Court Jester is an experimental MCP server for verifying AI-written code inside an agent loop.

It is built for a narrow job:

1. an agent writes code
2. Court Jester tries to disprove that code quickly
3. the agent gets a concrete repro and another repair attempt

Court Jester is strongest today on Python and TypeScript. It is not a general CI replacement, and it is not a secure hidden-judge system.

## Current Status

This repo is publishable as an experimental verifier and benchmark harness.

It is not yet proven ready for broad end-user release.

The current evidence supports:

- a working MCP verifier with `analyze`, `lint`, `execute`, and `verify`
- a benchmark harness for `baseline`, `required-final`, and `repair-loop` comparisons
- a small known-good control corpus that currently passes after a TypeScript alias-resolution fix
- benchmark evidence that Court Jester can improve outcomes for weaker models on the established suite

The current evidence does not yet support:

- a broad claim that Court Jester improves frontier-model outcomes in general
- a claim that false positives are already well-characterized across a large known-good corpus
- a production-readiness claim for arbitrary repos or agent workflows

The release bar and current read are documented in [release-readiness-private-beta.md](/Users/spencerlee/court-jester-mcp/docs/release-readiness-private-beta.md).

## What The Server Exposes

- `analyze`: tree-sitter-based function, class, import, and complexity extraction
- `lint`: Python via `ruff`, TypeScript via `biome`
- `execute`: sandboxed subprocess execution with memory and timeout limits
- `verify`: parse, lint, synthesize, execute, and optional test execution in one verdict

The MCP server entrypoint is in [main.rs](/Users/spencerlee/court-jester-mcp/src/main.rs), and the tool parameter definitions live in [lib.rs](/Users/spencerlee/court-jester-mcp/src/lib.rs).

## Repo Layout

- [src/](/Users/spencerlee/court-jester-mcp/src): Rust MCP server and tool implementations
- [tests/](/Users/spencerlee/court-jester-mcp/tests): Rust integration tests
- [bench/](/Users/spencerlee/court-jester-mcp/bench): benchmark harness, fixtures, evaluators, and model/provider adapters
- [docs/](/Users/spencerlee/court-jester-mcp/docs): design notes, benchmark writeups, and release planning

## Requirements

- Rust toolchain: use a current stable toolchain
- Python 3 for the benchmark harness
- `ruff` and `biome` if you want lint stages to run locally
- `bun` for the TypeScript benchmark fixtures that use Bun test commands

Toolchain note:

- In this environment, `cargo test` under Cargo `1.83.0` is not reliable because `rmcp-macros` requires `edition2024`.
- Use a newer stable toolchain before treating test results as authoritative.

## Quick Start

Start the MCP server over stdio:

```bash
cargo run
```

Run the Rust test suite with a current stable toolchain:

```bash
rustup run stable cargo test
```

Validate the benchmark matrix:

```bash
python -m bench.run_matrix --dry-run
```

Summarize benchmark output:

```bash
python -m bench.summarize_runs <output-dir>
```

More benchmark detail is in [bench/README.md](/Users/spencerlee/court-jester-mcp/bench/README.md).

## Benchmark Positioning

The benchmark harness exists to answer a product question, not just a unit-test question:

> Does `repair-loop` improve final task success over `baseline` without creating enough false positives or instability to make the verifier net harmful?

That means the important comparisons in this repo are:

- `baseline` vs `repair-loop`
- hidden-eval success, not only verify failures
- known-good false-positive control under `required-final`

Recent benchmark and design docs:

- [docs/README.md](/Users/spencerlee/court-jester-mcp/docs/README.md)
- [court-jester-overview.md](/Users/spencerlee/court-jester-mcp/docs/court-jester-overview.md)
- [benchmark-2026-03-26.md](/Users/spencerlee/court-jester-mcp/docs/benchmark-2026-03-26.md)
- [release-readiness-private-beta.md](/Users/spencerlee/court-jester-mcp/docs/release-readiness-private-beta.md)

## Model Providers In The Harness

The benchmark harness currently supports:

- local `codex exec`
- local `claude -p`
- deterministic replay and noop providers
- OpenAI-compatible chat endpoints via `openai_compat_chat`

The repo already includes Actual-backed manifests in [actual-api-qwen3-14b.json](/Users/spencerlee/court-jester-mcp/bench/models/actual-api-qwen3-14b.json) and [actual-api-qwen3-vl-30b.json](/Users/spencerlee/court-jester-mcp/bench/models/actual-api-qwen3-vl-30b.json).

## Publishing Guidance

If you publish this repo now, the honest framing is:

- experimental
- research / benchmark-driven
- private-beta-prep

The dishonest framing would be:

- production-ready verifier
- broadly proven for all users
- complete CI replacement
