# Court Jester

**An MCP server that catches bugs in AI-generated Python and TypeScript before they ship.**

AI coding agents produce plausible code fast, but they stop too early. Court Jester sits inside the agent loop, fuzzes every function the agent writes, and hands back a concrete failing input when something breaks. The agent gets a second chance to fix it — and the fix rate goes up.

```
Agent writes code  -->  Court Jester fuzzes it  -->  Bug found?
                                                      |
                                          yes: agent repairs with concrete repro
                                          no:  code ships
```

### Benchmarked results

On a 39-task suite (117 runs per configuration), adding Court Jester's repair loop to baseline generation:

| Model | Without Court Jester | With Court Jester | Tasks saved |
|-------|---------------------|-------------------|-------------|
| Claude | 106 / 117 (91%) | 116 / 117 (99%) | +10 |
| Codex | 108 / 117 (92%) | 117 / 117 (100%) | +9 |

The false-positive control (known-good code, 20 runs) passes 20/20.

---

## Get started

### 1. Install

```bash
# Requires Rust 1.85+ — install via https://rustup.rs if needed
cargo install --git https://github.com/slee1996/court-jester-mcp.git
```

This builds the binary and puts it on your PATH. Pre-built binaries are also available on [GitHub Releases](https://github.com/slee1996/court-jester-mcp/releases).

Optional tools (Court Jester works without them — lint becomes advisory):
- [ruff](https://docs.astral.sh/ruff/installation/) (Python lint)
- [biome](https://biomejs.dev/guides/getting-started/) (TypeScript lint)
- [bun](https://bun.sh) (required for TypeScript fuzz execution)

### 2. Connect to your agent

<details>
<summary><strong>Claude Code</strong></summary>

```bash
claude mcp add court-jester -- court-jester-mcp
```

Use `-s project` instead of the default to commit the config into your repo for your team.
</details>

<details>
<summary><strong>Codex CLI</strong></summary>

```bash
codex mcp add court-jester -- court-jester-mcp
```
</details>

<details>
<summary><strong>Cursor</strong></summary>

Add to `.cursor/mcp.json` in your project root (or `~/.cursor/mcp.json` for global):

```json
{
  "mcpServers": {
    "court-jester": {
      "command": "court-jester-mcp"
    }
  }
}
```
</details>

<details>
<summary><strong>VS Code / Copilot</strong></summary>

Add to `.vscode/mcp.json` in your workspace:

```json
{
  "servers": {
    "court-jester": {
      "command": "court-jester-mcp"
    }
  }
}
```
</details>

<details>
<summary><strong>Windsurf</strong></summary>

Add to `~/.codeium/windsurf/mcp_config.json`:

```json
{
  "mcpServers": {
    "court-jester": {
      "command": "court-jester-mcp"
    }
  }
}
```
</details>

<details>
<summary><strong>Cline</strong></summary>

Open Cline settings in VS Code, go to MCP Servers, and add:

```json
{
  "mcpServers": {
    "court-jester": {
      "command": "court-jester-mcp"
    }
  }
}
```
</details>

<details>
<summary><strong>Any other MCP host</strong></summary>

Court Jester is a stdio MCP server. The standard config shape works everywhere:

```json
{
  "command": "court-jester-mcp"
}
```
</details>

### 3. Add one line to your agent prompt

```text
After every code change, call court-jester `verify` on each changed file.
If verify returns overall_ok: false, fix the failing repro and verify again.
```

That's it. The agent now self-corrects.

### Example prompts to try

Once connected, try asking your agent:

- "Implement a URL parser and verify it handles edge cases before you're done."
- "Fix the bug in auth.py. Run court-jester verify after your fix to make sure it holds."
- "Refactor the payment module. Only verify the functions you changed — pass the diff."
- "Write a TypeScript date formatter. Use court-jester to fuzz it before committing."
- "Check if src/utils.py has any functions that crash on unexpected inputs."

### What this looks like in practice

**Claude Code** — you ask it to fix a bug. It writes the fix, calls `verify`, gets a fuzz failure, and repairs before responding:

```
You: Fix the timezone handling in profile.py

Agent: [edits profile.py]
Agent: [calls court-jester verify on profile.py]

  verify returned overall_ok: false
  execute stage: CRASH normalize_timezone(["  ", "UTC+99"])
    → ValueError: invalid timezone offset

Agent: [fixes normalize_timezone to validate offset range]
Agent: [calls court-jester verify again]

  verify returned overall_ok: true

Agent: Fixed. The original code crashed on invalid timezone strings
       like "UTC+99" — added offset validation.
```

**Codex CLI** — same loop, autonomous. Add the verify instruction to your AGENTS.md or system prompt and Codex calls it after every edit:

```
Task: Implement parse_csv_row() that handles quoted fields

Agent: [writes parse_csv_row in parser.py]
Agent: [calls court-jester verify on parser.py]

  verify returned overall_ok: false
  execute stage: CRASH parse_csv_row(['field,"has,comma",end'])
    → IndexError: list index out of range

Agent: [fixes quoted-field splitting logic]
Agent: [calls court-jester verify again]

  verify returned overall_ok: true
```

**Key point:** the agent never saw a test for `"has,comma"` — Court Jester synthesized that input from the function signature. Without it, the bug ships silently.

---

## How it works

`verify` runs five stages in order and returns a single pass/fail verdict:

| Stage | What it does | Fails the run? |
|-------|-------------|----------------|
| **parse** | Tree-sitter AST extraction | Yes — syntax errors stop the pipeline |
| **complexity** | Cyclomatic complexity check | Only when threshold is set |
| **lint** | `ruff` (Python) / `biome` (TypeScript) | No — advisory only |
| **execute** | Synthesize fuzz inputs from function signatures, run in sandbox | Yes |
| **test** | Run caller-supplied test file | Yes |

The **execute** stage is the differentiator. Court Jester walks tree-sitter output for every function, resolves parameter types (across imports, class fields, type aliases), generates diverse inputs, and calls each function in a sandboxed subprocess with time and memory limits. When a call crashes, the harness returns a structured failure with the exact input that triggered it.

This is not "ruff + biome in a trench coat." It finds runtime bugs that linters cannot.

### The repair loop

```
edit code
  --> verify(changed files, diff)
  --> overall_ok?
        yes: done
        no:  read first failing stage + repro
             fix code to handle that input
             verify again
```

Pass the `diff` parameter to restrict fuzzing to functions you actually changed — faster and avoids pre-existing failures.

---

## Try it now (no agent required)

```bash
court-jester-mcp verify \
    --file bench/repos/mini_py_service/profile.py \
    --language python \
    --project-dir bench/repos/mini_py_service
```

This will **fail** — the bundled sample has a latent `IndexError` that the fuzzer finds. That's the point.

---

## Tool reference

All tools accept **either** `code` (inline source) **or** `file_path` (absolute path), never both. Prefer `file_path` for code with local imports.

### `verify`

The primary tool. Parses, lints, fuzzes, and optionally runs tests in one call.

> "Verify src/parser.py after my changes."
> "Verify only the functions I touched — here's the diff."
> "Verify this file and fail if any function has cyclomatic complexity above 10."

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `file_path` | string | one of | Absolute path to source file |
| `code` | string | one of | Inline source |
| `language` | `"python"` / `"typescript"` | yes | Target language |
| `test_file_path` | string | no | Test file for authoritative test stage |
| `test_code` | string | no | Inline test code |
| `project_dir` | string | no | Root for `.venv` / `node_modules` resolution |
| `config_path` | string | no | Explicit Ruff/Biome config path for the lint stage |
| `virtual_file_path` | string | no | Virtual lint path for inline code so path-based rules still apply |
| `diff` | string | no | Unified diff — only fuzz functions touching changed lines |
| `complexity_threshold` | integer | no | Fail if any function exceeds this complexity |
| `output_dir` | string | no | Write timestamped JSON report to this directory |

### `analyze`

Extract functions, classes, imports, and complexity from a file without running anything.

> "Analyze utils.ts and show me all the exported functions and their complexity."
> "Which functions in this file overlap with my diff?"

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `file_path` / `code` | string | one of | Source to analyze |
| `language` | `"python"` / `"typescript"` | yes | Target language |
| `complexity_threshold` | integer | no | Adds complexity violations to result |
| `diff` | string | no | Adds `changed_functions` to result |

### `lint`

Run ruff (Python) or biome (TypeScript) and return diagnostics.

> "Lint src/handlers.py before I commit."
> "Run biome on this TypeScript file and show me what to fix."

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `file_path` / `code` | string | one of | Source to lint |
| `language` | `"python"` / `"typescript"` | yes | Picks `ruff` vs `biome` |
| `project_dir` | string | no | Root for config discovery and project-local linter binaries |
| `config_path` | string | no | Explicit Ruff/Biome config path |
| `virtual_file_path` | string | no | Virtual lint path for inline code so path-based rules still apply |

Lint runs in the user project context. Binary resolution order is:

1. Project-local binary (`.venv/bin/ruff`, `venv/bin/ruff`, or `node_modules/.bin/biome`)
2. Bundled sibling binary next to `court-jester-mcp`
3. `PATH`

When you pass inline `code`, set `virtual_file_path` if your lint config depends on filename or path overrides. Python uses Ruff's stdin filename hint. TypeScript materializes a temporary file at that path inside `project_dir`, runs Biome, then removes it.

### `execute`

Run a file in a sandboxed subprocess with time and memory limits. Use this to test a specific script directly rather than fuzzing function signatures.

> "Run this script in the sandbox with a 5-second timeout."
> "Execute my test harness and check if it passes."

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `file_path` / `code` | string | one of | Source to run |
| `language` | `"python"` / `"typescript"` | yes | Target runtime |
| `timeout_seconds` | number | no | Default `10.0` |
| `memory_mb` | integer | no | Default `128` |
| `project_dir` | string | no | Root for `.venv` / `node_modules` resolution |

### Verify response shape

```jsonc
{
  "stages": [
    {
      "name": "parse",
      "ok": true,
      "duration_ms": 12,
      "detail": { /* stage-specific */ },
      "error": null
    }
  ],
  "overall_ok": true,         // true only if every non-advisory stage passed
  "report_path": null          // set when output_dir is provided
}
```

On `overall_ok: false`, the first stage with `ok: false` is the one to act on. For the execute stage, `detail.fuzz_failures[]` contains the exact function, input, error type, and message.

<details>
<summary>Example: failing verify (fuzz found a crash)</summary>

```jsonc
{
  "stages": [
    { "name": "parse", "ok": true, "duration_ms": 1 },
    { "name": "lint",  "ok": true, "duration_ms": 14 },
    {
      "name": "execute",
      "ok": false,
      "duration_ms": 22,
      "detail": {
        "exit_code": 1,
        "stdout": "  CRASH normalize_display_name(['\\xa0...']): IndexError: string index out of range\n",
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

</details>

### Error responses

When a tool rejects its input before running, it returns:

```json
{
  "error": "Cannot read '/missing.py': No such file or directory (os error 2)",
  "error_kind": "read_failed"
}
```

`error_kind` values: `read_failed`, `ambiguous_input`, `missing_input`, `unsupported_language`.

---

## Configuration

### Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `COURT_JESTER_MAX_CONCURRENT_EXEC` | `1` | Concurrent sandbox subprocess cap |
| `COURT_JESTER_VERIFY_PYTHON_TIMEOUT_SECONDS` | `10` | Python fuzz timeout |
| `COURT_JESTER_VERIFY_TYPESCRIPT_TIMEOUT_SECONDS` | `25` | TypeScript fuzz timeout |
| `COURT_JESTER_VERIFY_TEST_TIMEOUT_SECONDS` | `30` | Test stage timeout |

Set via `env` in your MCP host config.

### Persistent reports

Set `output_dir` (MCP) or `--output-dir` (CLI) to write timestamped JSON reports:

```bash
court-jester-mcp verify --file src/profile.py --language python \
    --output-dir .court-jester/reports
```

### Release bundle (self-contained)

For environments without `ruff`/`biome` on `PATH`:

```bash
python scripts/prepare_release.py --release --require-ruff --require-biome
# produces dist/court-jester-release/ with court-jester-mcp + ruff + biome
```

---

## Troubleshooting

| Problem | Fix |
|---------|-----|
| `parse_error: true` on valid code | Wrong `language` field — check `.py` vs `.ts` |
| Execute stage times out | Raise `COURT_JESTER_VERIFY_PYTHON_TIMEOUT_SECONDS` or the TS equivalent |
| `memory_error: true` | Code exceeded 512 MB sandbox cap. For `execute` tool, raise `memory_mb` |
| `lint` shows `unavailable: true` | `ruff`/`biome` not found in the project, next to the binary, or on `PATH` — advisory, does not fail verify |
| `cargo build` fails on `edition2024` | Homebrew cargo is 1.83. Use `rustup` or `just build` |
| `cargo run` hangs | Expected — it's a stdio server waiting for MCP client. Use `just smoke` to test |
| Agent only verifies some functions | Diff-aware mode. Clear `diff` parameter to fuzz all functions |

---

## Development

```bash
just build            # cargo build --release
just test             # cargo test
just fmt              # cargo fmt
just smoke            # MCP handshake + tools/list
just smoke-sample     # handshake + real verify call
just verify-sample    # one-shot CLI verify
just bench-dry-run    # validate benchmark matrix
```

### Repo layout

```
src/        Rust MCP server, CLI, and tool implementations
tests/      Integration tests (one file per tool)
scripts/    smoke_mcp.py (minimal stdio MCP client), prepare_release.py
bench/      Python benchmark harness, fixtures, evaluators
docs/       Design notes, benchmark writeups
```

---

## Status

Court Jester is in **private beta**. It is a working MCP verifier with real benchmark evidence, not a production-hardened tool for arbitrary repos.

**What the evidence supports:**
- Repair loop beats baseline on both Claude and Codex across a 39-task suite
- Known-good control corpus passes cleanly (no false-positive blocker)
- Harness correctly distinguishes provider outages from code-quality failures

**What the evidence does not yet support:**
- Production-readiness for arbitrary repos or workflows
- Complete false-positive characterization beyond the current known-good corpus

Full release-readiness assessment: [docs/release-readiness-private-beta.md](docs/release-readiness-private-beta.md)

## Further reading

- [docs/court-jester-overview.md](docs/court-jester-overview.md) — why Court Jester exists
- [docs/system-flow.md](docs/system-flow.md) — architecture and runner flow
- [docs/tool-flow-diagram.md](docs/tool-flow-diagram.md) — flow diagram
- [docs/benchmark-2026-04-10.md](docs/benchmark-2026-04-10.md) — current benchmark results
- [bench/README.md](bench/README.md) — benchmark harness documentation
- [AGENTS.md](AGENTS.md) — contributor and agent guidelines

## License

[MIT](LICENSE)
