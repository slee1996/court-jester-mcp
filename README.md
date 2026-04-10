# Court Jester MCP

Court Jester is an experimental MCP server for disproving AI-written code inside an agent loop.

The job is narrow on purpose:

1. an agent writes code
2. Court Jester tries to disprove it quickly with a synthesized fuzz harness
3. the agent gets a concrete failing repro and another repair attempt

Court Jester works on **Python** and **TypeScript**. It is not a general CI replacement, and it is not a secure hidden-judge system. See [Status](#status) for an honest read of what it is and is not proven to do.

Current strongest evidence:

- clean `core-current` run:
  - `claude-default`: `106 / 117` baseline -> `116 / 117` repair-loop
  - `codex-default`: `108 / 117` baseline -> `117 / 117` repair-loop
- clean known-good control:
  - `20 / 20` success under `noop + required-final`
- current caveat:
  - fresh Codex and Spark reruns on `2026-04-10` are provider-outage-contaminated and currently fail as fast `provider_infra_error`, not code-quality misses

## Contents
- [How it works](#how-it-works)
- [Install](#install)
- [Quickstart](#quickstart)
- [Connect it to your agent](#connect-it-to-your-agent)
- [Tool reference](#tool-reference)
- [Verify response shape](#verify-response-shape)
- [Diff-aware verify](#diff-aware-verify)
- [Persistent reports](#persistent-reports)
- [Repair loop](#repair-loop)
- [Environment variables](#environment-variables)
- [Troubleshooting](#troubleshooting)
- [Development commands](#development-commands)
- [Repo layout](#repo-layout)
- [Benchmark Evidence](#benchmark-evidence)
- [Status](#status)
- [Further reading](#further-reading)
- [License](#license)

## How it works

`verify` is the main entry point. It runs up to five stages in order and returns a single `VerificationReport`:

| # | Stage        | What it does                                                                          | Fails overall? |
|---|--------------|---------------------------------------------------------------------------------------|---------------|
| 1 | `parse`      | Tree-sitter AST: functions, classes, imports, cyclomatic complexity                    | Yes (syntax errors short-circuit the rest of the pipeline) |
| 2 | `complexity` | Flags any function whose complexity exceeds the caller-supplied threshold              | Yes (only when `complexity_threshold` is set) |
| 3 | `lint`       | `ruff` for Python, `biome` for TypeScript                                              | No — diagnostics are advisory. A missing runner sets `unavailable: true` and does not fail. Only a *running* linter that crashes fails the stage. |
| 4 | `execute`    | **Synthesize** argument-level fuzz calls from the extracted signatures and types, then run them in a sandboxed subprocess. This is the step that actually disproves code. | Yes |
| 5 | `test`       | Optional. Runs a caller-supplied test file as an authoritative stage.                   | Yes |

The **synthesize** step is the differentiator vs. "ruff + biome in a trench coat." Court Jester walks the tree-sitter output for every function in scope, resolves parameter types (including across imports, class fields, and Python type aliases), generates a diverse set of inputs per parameter, and calls the functions under a harness that captures crashes, assertion failures, and time/memory limits. When a call raises, the harness emits a structured failure record with the input that triggered it, and `verify` echoes that record back in `stages[execute].detail.fuzz_failures`.

For a deeper architectural walkthrough see [docs/system-flow.md](docs/system-flow.md) and [docs/tool-flow-diagram.md](docs/tool-flow-diagram.md).

## Install

### Prerequisites

- **Rust 1.85+** (MSRV). `rmcp-macros` requires `edition2024`. Homebrew's stock `cargo` is 1.83 and **will fail to build** — install via [rustup](https://rustup.rs/) and make sure `~/.cargo/bin` comes before `/opt/homebrew/bin` on `PATH`. The `justfile` at the repo root does this for you.
- **Python 3.10+** for the benchmark harness and the `scripts/smoke_mcp.py` smoke client.
- **[ruff](https://docs.astral.sh/ruff/installation/)** for the Python lint stage: `pip install ruff` or `brew install ruff`. Alternatively, stage a sibling `ruff` next to the binary via `scripts/prepare_release.py`.
- **[biome](https://biomejs.dev/guides/getting-started/)** for the TypeScript lint stage: `npm i -g @biomejs/biome` or `brew install biome`. Alternatively, stage a sibling `biome` next to the binary via `scripts/prepare_release.py` — see [Release bundle with sibling Ruff and Biome binaries](#release-bundle-with-sibling-ruff-and-biome-binaries).
- **[bun](https://bun.sh)** for TypeScript fuzz execution and benchmark fixtures that use Bun test commands: `curl -fsSL https://bun.sh/install | bash`.

When `ruff` or `biome` is missing, the `lint` stage sets `unavailable: true` and the pipeline treats lint as advisory (the missing runner does not fail `verify`). Only a *running* linter that crashes is counted as a lint stage failure. Fuzz execution for TypeScript still requires `bun` on `PATH`.

### Build

```bash
cargo build --release
# binary at ./target/release/court-jester-mcp
```

Or via `just`:

```bash
just build
```

Install the binary somewhere on `PATH`:

```bash
cargo install --path .
```

### Release bundle with sibling Ruff and Biome binaries

For deployments where `ruff` or `biome` may not be on the target's `PATH`, you can stage a release
directory that carries its own Python and TypeScript linters next to the binary:

```bash
python scripts/prepare_release.py --release --require-ruff --require-biome
```

That produces `dist/court-jester-release/` with:

- `court-jester-mcp`
- `ruff`
- `biome`

At runtime the lint stage prefers sibling `./ruff` and `./biome` binaries next to its own
executable before falling back to `PATH`. Point your MCP host's `command` at the bundled binary.

## Quickstart

The fastest way to try Court Jester without wiring anything up is the new **one-shot CLI**:

```bash
./target/release/court-jester-mcp verify \
    --file bench/repos/mini_py_service/profile.py \
    --language python \
    --project-dir bench/repos/mini_py_service
```

Or simply:

```bash
just verify-sample
```

That prints a full `VerificationReport` as JSON. On the bundled fixture it will deliberately **fail** because the sample code has a latent `IndexError` that the fuzz stage finds.

### Subcommands

| Subcommand | Purpose                                                             |
|------------|---------------------------------------------------------------------|
| *(no args)*| Run as a stdio MCP server — default behavior used by agent hosts    |
| `verify`   | Full pipeline, prints a `VerificationReport` JSON to stdout         |
| `analyze`  | Tree-sitter analysis of a single file                               |
| `lint`     | Ruff or Biome against a single file                                 |
| `execute`  | Run a file in the sandbox with resource limits                      |
| `--help`   | Usage                                                               |
| `--version`| `court-jester-mcp <version>`                                        |

Exit code is `0` on pass and `1` on a failing verify/lint/execute result, so the CLI composes cleanly with shells and CI.

### Smoke-test the MCP server

```bash
just smoke           # handshake + tools/list
just smoke-sample    # handshake + real verify call against the bundled fixture
```

Both are thin wrappers around `python scripts/smoke_mcp.py`. The `--verify-sample` flag makes the smoke script hands-off — no paths to remember.

## Connect it to your agent

Court Jester is a stdio MCP server. Your agent host starts the process, talks JSON-RPC over stdin/stdout, and exposes the four tools to the model. Running `cargo run` by hand will appear to hang — that is expected: it is waiting for an MCP client to connect.

Most MCP hosts need the same core fields:

- `command`: path to `target/release/court-jester-mcp`
- `args`: usually empty
- `cwd`: optional, typically the repo the agent is working in
- `env`: optional, see [Environment variables](#environment-variables)

Minimal shape:

```json
{
  "command": "/absolute/path/to/court-jester-mcp",
  "args": [],
  "cwd": "/absolute/path/to/your/repo"
}
```

For local development, launching through Cargo also works:

```json
{
  "command": "cargo",
  "args": ["run", "--quiet"],
  "cwd": "/absolute/path/to/court-jester-mcp"
}
```

Prefer the compiled binary in real agent setups — faster startup, less toolchain variability.

The MCP protocol version used by the smoke client is `2025-06-18`.

### Codex CLI

```bash
codex mcp add court-jester -- /absolute/path/to/court-jester-mcp
codex mcp list
codex mcp get court-jester --json
codex -C /absolute/path/to/your/repo
```

### Claude Code

```bash
claude mcp add -s local court-jester -- /absolute/path/to/court-jester-mcp
# use `-s project` instead of `-s local` to commit the config into your repo
claude mcp list
claude mcp get court-jester
cd /absolute/path/to/your/repo
claude
```

### Agent prompt

Whatever host you use, the prompt is the same shape:

```text
Fix the bug. After every patch, call the court-jester `verify` tool on each
changed Python or TypeScript file before you finish. If `verify` returns
overall_ok: false, treat the failing stage and repro as authoritative: repair
the code so the repro no longer fails, and call `verify` again.
```

## Tool reference

All four tools accept **either** `code` (inline source) **or** `file_path` (absolute path on disk), never both. Prefer `file_path` for anything touching local imports, `node_modules`, or a `.venv`.

### `verify`

| Parameter              | Type               | Required | Description |
|------------------------|--------------------|----------|-------------|
| `file_path`            | string             | one of   | Absolute path to the source file |
| `code`                 | string             | one of   | Inline source (use for in-memory content not yet on disk) |
| `language`             | `"python"` / `"typescript"` | yes | Target language |
| `test_file_path`       | string             | no       | Test file to run as an authoritative stage |
| `test_code`            | string             | no       | Inline test code; ignored if `test_file_path` is set |
| `project_dir`          | string             | no       | Root for `.venv` / `node_modules` resolution. Auto-detected from `file_path` if omitted |
| `diff`                 | string (unified diff) | no    | Only fuzz functions touching changed lines — see [Diff-aware verify](#diff-aware-verify) |
| `complexity_threshold` | integer            | no       | Fails the run if any function exceeds this cyclomatic complexity |
| `output_dir`           | string             | no       | Write a timestamped JSON report to this directory — see [Persistent reports](#persistent-reports) |

### `analyze`

| Parameter              | Type               | Required | Description |
|------------------------|--------------------|----------|-------------|
| `file_path` / `code`   | string             | one of   | Source to analyze |
| `language`             | `"python"` / `"typescript"` | yes | Target language |
| `complexity_threshold` | integer            | no       | Adds `complexity_violations` and `complexity_ok` to the result |
| `diff`                 | string             | no       | Adds `changed_functions` to the result (functions overlapping changed lines) |

### `lint`

| Parameter            | Type               | Required | Description |
|----------------------|--------------------|----------|-------------|
| `file_path` / `code` | string             | one of   | Source to lint |
| `language`           | `"python"` / `"typescript"` | yes | Picks `ruff` vs `biome` |

### `execute`

| Parameter          | Type               | Required | Description |
|--------------------|--------------------|----------|-------------|
| `file_path` / `code` | string           | one of   | Source to run |
| `language`         | `"python"` / `"typescript"` | yes | Target runtime |
| `timeout_seconds`  | number             | no       | Default `10.0` |
| `memory_mb`        | integer            | no       | Default `128` |
| `project_dir`      | string             | no       | Root for `.venv` / `node_modules` resolution. Auto-detected from `file_path` if omitted |

### Pre-tool validation errors

When a tool rejects its input before running (missing file, ambiguous `code` + `file_path`, unsupported language, unreadable source), it returns a uniform JSON string:

```json
{
  "error": "Cannot read '/missing.py': No such file or directory (os error 2)",
  "error_kind": "read_failed"
}
```

`error_kind` is one of `read_failed`, `ambiguous_input`, `missing_input`, `unsupported_language`. Stage-level failures inside a successful tool call live on `stages[i].error`, not at the top level.

## Verify response shape

Every successful `verify` call returns a `VerificationReport`:

```jsonc
{
  "stages": [
    {
      "name": "parse" | "complexity" | "lint" | "execute" | "test",
      "ok": true,
      "duration_ms": 12,
      "detail": { /* stage-specific JSON */ },
      "error": null
    }
  ],
  "overall_ok": true,
  "report_path": null  // string when output_dir is set, see Persistent reports
}
```

- `overall_ok` is `true` only if every non-advisory stage passed.
- `stages[].detail` is stage-specific: the `parse` stage embeds the full `AnalysisResult`, `lint` embeds the `LintResult`, `execute` embeds the sandbox `ExecutionResult` plus an optional `fuzz_failures` array, and `test` embeds another `ExecutionResult`.
- `stages[].error` is populated on failure with human-readable text (stderr for execute/test, a short explanation otherwise).

### Example: passing verify

```console
$ court-jester-mcp verify --file good_profile.py --language python
```

```json
{
  "stages": [
    { "name": "parse", "ok": true, "duration_ms": 1, "detail": { "functions": [ /* ... */ ], "parse_error": false } },
    { "name": "lint",  "ok": true, "duration_ms": 8, "detail": { "diagnostics": [] } },
    {
      "name": "execute",
      "ok": true,
      "duration_ms": 24,
      "detail": {
        "exit_code": 0,
        "timed_out": false,
        "memory_error": false,
        "stdout": "FUZZ normalize_display_name: 30 passed, 0 rejected (of 30)\nAll fuzz tests passed\n",
        "stderr": ""
      }
    }
  ],
  "overall_ok": true
}
```

### Example: failing verify (fuzz found a crash)

```console
$ court-jester-mcp verify --file bench/repos/mini_py_service/profile.py --language python \
    --project-dir bench/repos/mini_py_service
```

```jsonc
{
  "stages": [
    { "name": "parse", "ok": true, "duration_ms": 1, "detail": { "functions": [/* ... */] } },
    { "name": "lint",  "ok": true, "duration_ms": 14, "detail": { "diagnostics": [] } },
    {
      "name": "execute",
      "ok": false,
      "duration_ms": 22,
      "detail": {
        "exit_code": 1,
        "timed_out": false,
        "memory_error": false,
        "stdout": "  CRASH normalize_display_name(['\\xa0...']): IndexError: string index out of range\nFUZZ normalize_display_name: 28 passed, 0 rejected, 2 CRASHED (of 30)\n",
        "stderr": "AssertionError: Fuzz testing failed: 1 function(s) had failures",
        "fuzz_failures": [
          {
            "function": "normalize_display_name",
            "input": "['\\xa0\\xa0\\xa0\\xa0...']",
            "error_type": "IndexError",
            "message": "string index out of range",
            "severity": "crash"
          }
        ]
      },
      "error": "AssertionError: Fuzz testing failed: 1 function(s) had failures"
    }
  ],
  "overall_ok": false
}
```

**How an agent should read this:** on `overall_ok: false`, find the first stage with `ok: false`. Its `detail.fuzz_failures[]` (for the execute stage) or `error` string is the authoritative repro. The repair attempt's job is to make that specific input stop failing.

## Diff-aware verify

Passing a unified diff string as the `diff` parameter restricts fuzzing to functions that overlap with changed lines. This is the right mode for a repair loop: the agent only changed a few functions, and rerunning the fuzzer on the entire file wastes time and can surface pre-existing failures that the agent did not cause.

```json
{
  "name": "verify",
  "arguments": {
    "file_path": "/repo/src/profile.py",
    "language": "python",
    "project_dir": "/repo",
    "diff": "diff --git a/src/profile.py b/src/profile.py\n@@ -12,6 +12,8 @@\n..."
  }
}
```

From the CLI you pass a diff via a file: `--diff-file changes.patch`.

## Persistent reports

Set `output_dir` (MCP) or `--output-dir` (CLI) to make `verify` write a timestamped JSON report alongside the in-memory response:

```bash
court-jester-mcp verify --file src/profile.py --language python \
    --output-dir .court-jester/reports
```

Each report file is named `<timestamp>-<basename>.json` and includes a `meta`, `stages`, `overall_ok`, and `summary` block. The written path is echoed back on `VerificationReport.report_path`.

## Repair loop

The intended loop is:

```text
edit files
call verify(changed files, diff)
if overall_ok:
    continue or finalize
else:
    take the first failing stage and its repro as authoritative
    require the next patch to change behavior on that repro
    call verify again
```

A concrete pattern that works well in practice:

1. run `verify` immediately after the patch, before the agent writes its final answer
2. treat the first failing stage and its repro as authoritative feedback
3. require the next attempt to change behavior on that specific repro
4. stop only after `verify` passes or the repair budget is exhausted

That is also how the benchmark harness uses Court Jester. The fuller benchmark-side client implementation lives in [bench/mcp_client.py](bench/mcp_client.py).

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `COURT_JESTER_MAX_CONCURRENT_EXEC`           | `1`  | Cap on concurrent sandbox subprocesses. Kept at 1 by default so parallel fuzz harnesses don't fight over on-disk artifacts. Increase for agent setups that can tolerate interleaving. |
| `COURT_JESTER_VERIFY_PYTHON_TIMEOUT_SECONDS`   | `10` | Execute-stage timeout for Python fuzz harnesses. |
| `COURT_JESTER_VERIFY_TYPESCRIPT_TIMEOUT_SECONDS` | `25` | Execute-stage timeout for TypeScript fuzz harnesses (bun/tsx cold start is slower). |
| `COURT_JESTER_VERIFY_TEST_TIMEOUT_SECONDS`     | `30` | Timeout for the optional test stage. |

All four are read at invocation time, so you can set them per-agent or per-call via `env` in your MCP host config.

## Troubleshooting

**`parse_error: true` on code that looks valid.** Usually the `language` field is wrong. `.ts` with `language: "python"` or `.py` with `language: "typescript"` will produce a parse error.

**`verify` hangs or the execute stage times out.** Your fuzz targets are slower than the defaults. Raise `COURT_JESTER_VERIFY_PYTHON_TIMEOUT_SECONDS` or `COURT_JESTER_VERIFY_TYPESCRIPT_TIMEOUT_SECONDS`. TypeScript in particular pays a bun/tsx startup cost per call.

**`memory_error: true` in the execute stage.** Your code allocated past the 512 MB sandbox cap. For the standalone `execute` tool, raise `memory_mb`. The verify pipeline's cap is not currently user-tunable.

**`lint` stage reports `unavailable: true`.** `ruff` / `biome` isn't on `PATH` (or alongside the binary). The lint stage treats this as advisory and does not fail `verify`. Install the missing tool (see [Prerequisites](#install)) or stage a sibling via `scripts/prepare_release.py`.

**`cargo build` complains about `edition2024`.** You are building with Homebrew's cargo 1.83. Install rustup and put `~/.cargo/bin` before `/opt/homebrew/bin` on `PATH`, or use `just build` which does that for you.

**The MCP server appears to hang when I run `cargo run`.** That is expected. It is a stdio server and is waiting for JSON-RPC on stdin. Use `just smoke` or `python scripts/smoke_mcp.py --release` to drive it from the outside.

**Agent only verifies some functions after a diff.** That is diff-aware mode doing its job — see [Diff-aware verify](#diff-aware-verify). Clear the `diff` parameter to fuzz every function in the file.

## Development commands

The canonical commands live in the `justfile`:

```bash
just build            # cargo build --release
just test             # cargo test
just fmt              # cargo fmt
just smoke            # handshake + tools/list
just smoke-sample     # handshake + real verify call against bundled fixture
just verify-sample    # one-shot verify via CLI
just bench-dry-run    # validate the benchmark matrix without running agents
just bench-summarize <dir>
```

Every recipe is a short shell one-liner — you can copy them straight out if you don't have `just` installed.

More benchmark detail is in [bench/README.md](bench/README.md). Repository conventions (code style, commit/PR guidelines) are in [AGENTS.md](AGENTS.md).

## Repo layout

- [src/](src) — Rust MCP server, CLI subcommands, and tool implementations
- [tests/](tests) — Rust integration tests, one file per tool surface
- [scripts/](scripts) — `smoke_mcp.py`, the minimal stdio MCP client
- [bench/](bench) — Python benchmark harness: fixtures, evaluators, runner, model/provider adapters
- [docs/](docs) — design notes, benchmark writeups, release planning
- [AGENTS.md](AGENTS.md) — repository guidelines for contributors and agents
- [justfile](justfile) — canonical build/test/run recipes

## Benchmark Evidence

The current benchmark story is stronger than the earlier six-task slice and should be the main public read.

### Clean utility result

The strongest clean release-evidence run currently available is the `core-current` matrix:

- tasks: `39`
- models: `claude-default`, `codex-default`
- policies: `baseline`, `repair-loop`
- repeats: `3`

That produces `117` runs per model-policy pair.

Results:

- `claude-default`
  - baseline: `106 / 117`
  - repair-loop: `116 / 117`
- `codex-default`
  - baseline: `108 / 117`
  - repair-loop: `117 / 117`

That is real lift, not just better bug observability:

- Claude improved by `+10` tasks
- Codex improved by `+9` tasks

On the clean core run, Claude reduced hidden semantic misses from `11` to `1`, and Codex reduced them from `8` plus `1` provider failure to `0`.

### False-positive control

The current known-good control corpus is still small, but it is clean:

- task set: `known-good-corpus`
- policy: `required-final`
- model: `noop`
- repeats: `10`
- result: `20 / 20`

That matters because an earlier run exposed a real TypeScript false-positive bug in alias handling. The current control corpus now passes after the synthesis fix.

### Provider-health caveat

Fresh same-day reruns are currently limited by provider stability, not by benchmark logic.

After adding early aborts, retries, and better failure classification, current reruns show:

- fresh Codex reruns failing broadly as fast `provider_infra_error`
- fresh Spark reruns failing the same way
- common signature: `Transport channel closed` plus `Internal server error`

This is the right outcome operationally because the harness now fails fast and labels the issue honestly, but it means fresh Codex/Spark reruns on `2026-04-10` are not clean quality evidence.

### What this means

The current evidence supports:

- Court Jester can improve final task success in an agent loop on a larger suite than the original six-task report
- the current known-good corpus does not show a false-positive blocker
- the harness now distinguishes provider outages from code-quality failures much better than before

The current evidence does not yet support:

- a claim that all frontier-model reruns are clean at any given moment
- a claim that false positives are fully characterized beyond the current small known-good corpus
- a production-readiness claim for arbitrary repos or workflows

## Status

Court Jester is **experimental / research-driven / private-beta-prep**. It is not a production-ready verifier, a broadly proven tool for all users, or a complete CI replacement.

What the current evidence supports:

- a working MCP verifier with `analyze`, `lint`, `execute`, and `verify`
- a benchmark harness for `baseline`, `required-final`, and `repair-loop` comparisons
- a clean larger benchmark result showing `repair-loop` beating `baseline` on both Claude and Codex
- a small known-good control corpus that currently passes cleanly

What the current evidence does **not** yet support:

- a broad claim that every fresh frontier-model rerun is stable and clean at provider time
- a claim that false positives are already well-characterized across a large known-good corpus
- a production-readiness claim for arbitrary repos or agent workflows

The release bar and honest read are documented in [docs/release-readiness-private-beta.md](docs/release-readiness-private-beta.md).

### Benchmark positioning

The benchmark harness exists to answer a product question, not just a unit-test question:

> Does `repair-loop` improve final task success over `baseline` without creating enough false positives or instability to make the verifier net harmful?

That means the important comparisons in this repo are:

- `baseline` vs `repair-loop`
- hidden-eval success, not just verify failure counts
- known-good false-positive control under `required-final`

## Further reading

- [docs/README.md](docs/README.md) — docs index
- [docs/court-jester-overview.md](docs/court-jester-overview.md) — why Court Jester exists and what the benchmark is meant to answer
- [docs/system-flow.md](docs/system-flow.md) — detailed architecture and runner flow
- [docs/tool-flow-diagram.md](docs/tool-flow-diagram.md) — compact flow diagram companion
- [docs/benchmark-2026-04-10.md](docs/benchmark-2026-04-10.md) — current strongest benchmark summary
- [docs/benchmark-2026-03-26.md](docs/benchmark-2026-03-26.md) — earlier six-task benchmark report
- [docs/big-benchmark-runbook.md](docs/big-benchmark-runbook.md) — commands and pass/fail criteria for the large release-evidence run
- [docs/release-readiness-private-beta.md](docs/release-readiness-private-beta.md) — release bar and current read

## License

Not yet set. Do not redistribute without permission from the author.
