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

**Download the release binary** (macOS Apple Silicon):

```bash
curl -L https://github.com/slee1996/court-jester-mcp/releases/latest/download/court-jester-v0.1.2-darwin-arm64.tar.gz | tar xz
# move the binary somewhere on PATH
mv court-jester-mcp /usr/local/bin/
```

Or **build from source** (any platform, requires Rust 1.85+):

```bash
git clone https://github.com/slee1996/court-jester-mcp.git
cd court-jester-mcp
cargo install --path .
```

Optional lint tools (advisory — Court Jester works without them):
- Python lint: `pip install ruff` or `brew install ruff`
- TypeScript lint: `npm i -g @biomejs/biome` or `brew install biome`
- TypeScript fuzz execution: [bun](https://bun.sh)

### 2. Connect to your agent

**Claude Code:**

```bash
claude mcp add court-jester -- /absolute/path/to/court-jester-mcp
```

**Codex CLI:**

```bash
codex mcp add court-jester -- /absolute/path/to/court-jester-mcp
```

**Any MCP host** (generic JSON config):

```json
{
  "command": "/absolute/path/to/court-jester-mcp"
}
```

### 3. Add one line to your agent prompt

```text
After every code change, call court-jester `verify` on each changed file.
If verify returns overall_ok: false, fix the failing repro and verify again.
```

That's it. The agent now self-corrects.

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

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `file_path` | string | one of | Absolute path to source file |
| `code` | string | one of | Inline source |
| `language` | `"python"` / `"typescript"` | yes | Target language |
| `test_file_path` | string | no | Test file for authoritative test stage |
| `test_code` | string | no | Inline test code |
| `project_dir` | string | no | Root for `.venv` / `node_modules` resolution |
| `diff` | string | no | Unified diff — only fuzz functions touching changed lines |
| `complexity_threshold` | integer | no | Fail if any function exceeds this complexity |
| `output_dir` | string | no | Write timestamped JSON report to this directory |

### `analyze`

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `file_path` / `code` | string | one of | Source to analyze |
| `language` | `"python"` / `"typescript"` | yes | Target language |
| `complexity_threshold` | integer | no | Adds complexity violations to result |
| `diff` | string | no | Adds `changed_functions` to result |

### `lint`

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `file_path` / `code` | string | one of | Source to lint |
| `language` | `"python"` / `"typescript"` | yes | Picks `ruff` vs `biome` |

### `execute`

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
| `lint` shows `unavailable: true` | `ruff`/`biome` not on `PATH` — advisory, does not fail verify |
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
