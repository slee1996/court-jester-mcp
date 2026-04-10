# Court Jester MCP

Court Jester is an experimental MCP server for verifying AI-written code inside an agent loop.

It is built for a narrow job:

1. an agent writes code
2. Court Jester tries to disprove that code quickly
3. the agent gets a concrete repro and another repair attempt

If you want the shortest possible description of how to use it:

1. build the `court-jester-mcp` binary
2. register that binary as a stdio MCP server in your agent/client
3. have the agent call `verify` on changed files before it declares success
4. if `verify` fails, feed the failing repro back into the next repair attempt

Court Jester is strongest today on Python and TypeScript. It is not a general CI replacement, and it is not a secure hidden-judge system.

## What The Server Exposes

- `analyze`: tree-sitter-based function, class, import, and complexity extraction
- `lint`: Python via `ruff`, TypeScript via `biome`
- `execute`: sandboxed subprocess execution with memory and timeout limits
- `verify`: parse, lint, synthesize, execute, and optional test execution in one verdict

The MCP server entrypoint is in [main.rs](src/main.rs), and the tool parameter definitions live in [lib.rs](src/lib.rs).

## Repo Layout

- [src/](src): Rust MCP server and tool implementations
- [tests/](tests): Rust integration tests
- [bench/](bench): benchmark harness, fixtures, evaluators, and model/provider adapters
- [docs/](docs): design notes, benchmark writeups, and release planning

## Requirements

- Rust toolchain: use a current stable toolchain
- Python 3 for the benchmark harness
- `ruff` if you want Python lint stages to run locally
- `biome` if you want TypeScript lint stages to run locally, unless you stage a sibling `biome` binary next to `court-jester-mcp`
- `bun` for the TypeScript benchmark fixtures that use Bun test commands

Toolchain note:

- In this environment, `cargo test` under Cargo `1.83.0` is not reliable because `rmcp-macros` requires `edition2024`.
- Use a newer stable toolchain before treating test results as authoritative.

## 5-Minute Setup

Court Jester is a stdio MCP server. You normally do not "open" it in a browser or call it over HTTP.

Your MCP client or agent runner starts the process, talks JSON-RPC over stdin/stdout, and exposes the tools to an agent.

If you run `cargo run` manually in a terminal, it will appear to hang. That is expected: it is waiting for an MCP client to connect and send requests.

Build a release binary:

```bash
cargo build --release
```

If you want the release artifact to carry its own TypeScript linter, stage a sibling `biome`
binary next to `court-jester-mcp`:

```bash
python scripts/prepare_release.py --release --require-biome
```

That creates `dist/court-jester-release/` with:

- `court-jester-mcp`
- `biome`

At runtime, Court Jester checks for `./biome` next to its own executable before falling back to
`biome` on `PATH`.

Smoke-test the MCP handshake and tool discovery:

```bash
python scripts/smoke_mcp.py --release
```

Expected output:

```text
Connected to court-jester 0.1.1
Tools:
- analyze
- execute
- lint
- verify
```

Run one real `verify` call against a fixture file:

```bash
python scripts/smoke_mcp.py \
  --release \
  --verify-file bench/repos/mini_py_service/profile.py \
  --language python \
  --project-dir bench/repos/mini_py_service \
  --test-file bench/repos/mini_py_service/tests/court_jester_public_verify.py
```

If that works, your MCP server is usable from an agent.

## Connect It To Your Agent

Most MCP clients need the same core fields:

- `command`: path to `target/release/court-jester-mcp`
- `args`: usually empty when launching the binary directly
- `cwd`: optional, typically this repo or the repo the agent is working in
- `env`: optional, only if you need to control tool discovery

A minimal shape looks like:

```json
{
  "command": "/absolute/path/to/court-jester-mcp",
  "args": [],
  "cwd": "/absolute/path/to/your/repo"
}
```

If you staged a release bundle with `scripts/prepare_release.py`, point `command` at the bundled
`court-jester-mcp` inside that directory. The sibling `biome` will be discovered automatically.

For local development, launching through Cargo is also fine:

```json
{
  "command": "cargo",
  "args": ["run", "--quiet"],
  "cwd": "/absolute/path/to/court-jester-mcp"
}
```

The exact config file location depends on the MCP host. The important part is that the host starts this process as a stdio server and exposes the four tools to the model.

For most agent setups, the built binary is better than `cargo run` because startup is faster and there is less toolchain variability.

### Codex CLI

Add Court Jester to Codex as a stdio MCP server:

```bash
codex mcp add court-jester -- /absolute/path/to/court-jester-mcp
```

Confirm that Codex sees it:

```bash
codex mcp list
codex mcp get court-jester --json
```

Then start Codex in the repo you want to work on and tell it to use Court Jester as a verify gate:

```bash
codex -C /absolute/path/to/your/repo
```

Example prompt:

```text
Fix the bug. After every patch, call court-jester verify on each changed Python or TypeScript file before you finish. If verify fails, treat the failing repro as authoritative, repair the code, and run verify again.
```

### Claude Code

Add Court Jester to Claude Code as a stdio MCP server:

```bash
claude mcp add -s local court-jester -- /absolute/path/to/court-jester-mcp
```

If you want the server config to live with the repo instead of only in your local Claude settings, use `-s project` instead of `-s local`.

Confirm that Claude Code sees it:

```bash
claude mcp list
claude mcp get court-jester
```

Then start Claude Code in the repo you want to work on:

```bash
cd /absolute/path/to/your/repo
claude
```

Example prompt:

```text
Fix the bug. Use the court-jester verify tool on every file you change before you finish. If verify reports a failing stage or repro, change the code to satisfy that repro and rerun verify.
```

In both Codex and Claude Code, the important behavior is the same:

- the agent edits files in the target repo
- the host exposes the `court-jester` MCP tools to the model
- the model calls `verify` before it declares success
- failing repros become the next repair input

## First Useful Call

Once your agent can see the server, it should see four tools:

- `analyze`
- `lint`
- `execute`
- `verify`

`verify` is the main entry point for an agent loop.

Use `file_path` for normal repo work. Use `code` only when you are verifying in-memory content that has not been written to disk yet.

Pass `project_dir` when the file relies on local imports, `node_modules`, or `.venv` resolution.

Pass `test_file_path` when you already have a focused regression test or public verify test you want included in the verdict.

Typical `verify` call for a Python file:

```json
{
  "name": "verify",
  "arguments": {
    "file_path": "/absolute/path/to/profile.py",
    "language": "python",
    "project_dir": "/absolute/path/to/repo",
    "test_file_path": "/absolute/path/to/tests/test_profile.py"
  }
}
```

Typical `verify` call for a TypeScript file:

```json
{
  "name": "verify",
  "arguments": {
    "file_path": "/absolute/path/to/src/semver.ts",
    "language": "typescript",
    "project_dir": "/absolute/path/to/repo",
    "test_file_path": "/absolute/path/to/tests/semver.test.ts"
  }
}
```

If you are building your own agent instead of using an MCP host, [scripts/smoke_mcp.py](scripts/smoke_mcp.py) is the smallest end-to-end example in the repo, and [bench/mcp_client.py](bench/mcp_client.py) is the fuller benchmark-side client.

## Use It In A Repair Loop

The intended loop is:

1. the agent edits one or more files
2. the agent calls `verify` on the changed file(s)
3. if `verify` passes, the agent can continue to broader checks or finish
4. if `verify` fails, the agent reads the failing stage and repro
5. the agent makes another repair attempt
6. the agent calls `verify` again

In pseudocode:

```text
edit files
call verify(changed files)
if verify passes:
    continue or finalize
else:
    use the failing repro as authoritative feedback
    repair code
    call verify again
```

A concrete pattern that works well in practice:

1. run `verify` immediately after the patch, before the agent writes its final answer
2. treat the failing stage and repro as authoritative
3. require the next attempt to change behavior on that repro
4. stop only after `verify` passes or the repair budget is exhausted

That is also how the benchmark harness uses Court Jester:

```text
model writes code -> verify -> repair attempt -> verify -> hidden evaluation
```

The benchmark-side client implementation lives in [bench/mcp_client.py](bench/mcp_client.py).

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

The release bar and current read are documented in [release-readiness-private-beta.md](docs/release-readiness-private-beta.md).

## Sanity Check

If you want to confirm the binary starts successfully before wiring it into an agent, run:

```bash
cargo run
```

You should not expect human-readable output. A successful startup means the process stays running and waits for MCP traffic over stdio.

## Development Commands

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

More benchmark detail is in [bench/README.md](bench/README.md).

## Benchmark Positioning

The benchmark harness exists to answer a product question, not just a unit-test question:

> Does `repair-loop` improve final task success over `baseline` without creating enough false positives or instability to make the verifier net harmful?

That means the important comparisons in this repo are:

- `baseline` vs `repair-loop`
- hidden-eval success, not only verify failures
- known-good false-positive control under `required-final`

Recent benchmark and design docs:

- [docs/README.md](docs/README.md)
- [court-jester-overview.md](docs/court-jester-overview.md)
- [benchmark-2026-03-26.md](docs/benchmark-2026-03-26.md)
- [release-readiness-private-beta.md](docs/release-readiness-private-beta.md)

## Model Providers In The Harness

The benchmark harness currently supports:

- local `codex exec`
- local `claude -p`
- deterministic replay and noop providers
- OpenAI-compatible chat endpoints via `openai_compat_chat`

The repo already includes Actual-backed manifests in [actual-api-qwen3-14b.json](bench/models/actual-api-qwen3-14b.json) and [actual-api-qwen3-vl-30b.json](bench/models/actual-api-qwen3-vl-30b.json).

## Publishing Guidance

If you publish this repo now, the honest framing is:

- experimental
- research / benchmark-driven
- private-beta-prep

The dishonest framing would be:

- production-ready verifier
- broadly proven for all users
- complete CI replacement
