# Court Jester

> **Experimental**: Court Jester is under active development. CLI flags, output fields, and behavior may still change.

**Court Jester is a CLI for making AI-generated Python and TypeScript code fail as fast as possible before the agent declares victory.**

AI agents are good at writing plausible code and bad at knowing when they are actually finished. Court Jester shows up the moment the code starts looking a little too sure of itself: it runs right after the edit, tries to break the changed file immediately, and turns "this looks done" into a concrete repro the agent can repair.

```text
agent edits code -> court-jester verify -> fast concrete failure?
                                        |
                             yes: repair from repro
                             no:  ship with more confidence
```

It is just a CLI. No MCP transport, editor plugin, or custom agent integration layer is required.

## Why Use It

- Finds runtime and semantic failures, not just style issues
- Produces concrete repros instead of vague "something seems wrong" feedback
- Fits into any agent loop or CI job because it shells out like any other CLI
- Uses the target project's Ruff and Biome config instead of detached temp-dir defaults
- Returns structured JSON so automation can make decisions on pass/fail

## Install

Fastest path:

```bash
curl -fsSL https://raw.githubusercontent.com/slee1996/court-jester-mcp/main/install.sh | sh
```

That installs `court-jester` into `~/.local/bin`.

The install script:

- downloads the latest release binary for your platform
- does not require a Rust toolchain
- does not require agent transport setup

If `~/.local/bin` is not on `PATH`, add:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

<details>
<summary>Build from source instead</summary>

```bash
cargo install --git https://github.com/slee1996/court-jester-mcp.git
```

The repo is currently pinned to Rust `1.86.0` via [`rust-toolchain.toml`](rust-toolchain.toml).
</details>

## Install Optional Tooling

Court Jester itself is one binary, but some stages rely on tools from the project you are checking:

- Python lint: [Ruff](https://docs.astral.sh/ruff/installation/)
- TypeScript lint: [Biome](https://biomejs.dev/guides/getting-started/)
- TypeScript execute/verify: [bun](https://bun.sh)

Tool resolution order:

1. Project-local binaries such as `.venv/bin/ruff`, `venv/bin/ruff`, and `node_modules/.bin/biome`
2. Optional sibling binaries next to `court-jester`
3. `PATH`

## Use It With An Agent

The simplest integration is to run `verify` after every edit to a changed file.

Agent command:

```bash
court-jester verify --file <changed-file> --language <python|typescript>
```

Prompt snippet:

```text
After every code change, run `court-jester verify --file <changed-file> --language <python|typescript>`.
If verify returns overall_ok: false, fix the failing repro and verify again.
Treat verify repros as authoritative.
```

If the repo has a local virtualenv, `node_modules`, or lint config, pass `--project-dir` so lint and execution resolve in the right project context:

```bash
court-jester verify \
  --file apps/api/profile.py \
  --language python \
  --project-dir .
```

## Use It Directly

Python:

```bash
court-jester verify \
  --file src/profile.py \
  --language python \
  --project-dir .
```

TypeScript with an authoritative test file:

```bash
court-jester verify \
  --file src/semver.ts \
  --language typescript \
  --project-dir . \
  --test-file tests/semver.test.ts
```

Write JSON reports to disk:

```bash
court-jester verify \
  --file src/profile.py \
  --language python \
  --output-dir .court-jester/reports
```

Other commands:

```text
court-jester analyze  [OPTIONS]
court-jester lint     [OPTIONS]
court-jester execute  [OPTIONS]
court-jester --help
```

## What `verify` Does

`verify` runs a staged pipeline and returns one JSON report.

| Stage | What it does | Fails the run? |
|-------|--------------|----------------|
| `parse` | Tree-sitter AST extraction | Yes |
| `complexity` | Optional complexity gate | Only if you set a threshold |
| `lint` | Ruff or Biome in the target project context | No, advisory only |
| `execute` | Synthesized fuzz/property checks in a sandbox | Yes |
| `test` | Optional caller-supplied test file | Yes |

The important stage is `execute`: Court Jester synthesizes a language-specific harness from the AST, runs it in a sandbox, and reports the concrete repro when something breaks.

- Python: generates direct calls and adversarial edge cases from the function surface, then treats both runtime exceptions and contract violations as execute-stage failures. That includes crashes like `TypeError`, `AttributeError`, `KeyError`, `IndexError`, `RecursionError`, `MemoryError`, `ValueError`, `ZeroDivisionError`, and `UnicodeError`, plus return-type mismatches, inconsistency, failed idempotency or boundedness checks, non-negative violations, nullish-string leaks, symmetry violations, comparator violations, and roundtrip failures for inferred encode/decode pairs.
- TypeScript: resolves local aliases, interfaces, classes, and imported types where it can, generates structured values for unions, arrays, records, nullable branches, and inline object shapes, then treats both runtime crashes and contract violations as execute-stage failures. That includes crashes like `TypeError`, `RangeError`, `ReferenceError`, `URIError`, and stack overflows, plus return-type mismatches, inconsistency, failed idempotency or boundedness checks, blank string outputs, nullish-string leaks, symmetry violations, comparator violations, and roundtrip failures for inferred encode/decode pairs.

## Common Flags

Core flags:

- `--file <PATH>`: source file to inspect
- `--language python|typescript`
- `--project-dir <PATH>`: root for `.venv`, `node_modules`, and config discovery
- `--config-path <PATH>`: explicit Ruff or Biome config path
- `--virtual-file-path <PATH>`: preserve lint path semantics for temp/generated code

Useful `verify` flags:

- `--test-file <PATH>`: add an authoritative test stage
- `--tests-only`: skip fuzz execute and run only the authoritative test stage
- `--output-dir <PATH>`: persist JSON reports
- `--diff-file <PATH>`: only inspect changed functions from a unified diff
- `--complexity-threshold <N>`: fail when a function exceeds the threshold

Sandbox flags for `execute`:

- `--timeout-seconds <F>`
- `--memory-mb <N>`

Use `court-jester --help` for the full CLI help text.

## Exit Codes And Output

- `0`: command succeeded and the code passed
- `1`: the code failed verification or execution, but Court Jester still returned structured JSON
- `2`: CLI usage or setup error

That makes it easy to use in:

- agent loops
- pre-merge checks
- local shell workflows
- benchmark harnesses

## Evidence

Court Jester's current headline benchmark is a repeated 39-task verify-only repair policy on `core-current`.

- Claude: `102 / 117` baseline -> `116 / 117` with verify-guided repair
- Codex: `107 / 117` baseline -> `116 / 117` with verify-guided repair
- Aggregate: `209 / 234` baseline -> `232 / 234` with verify-guided repair
- False-positive gauntlet: `270 / 270` passes (`80 / 80` local known-good, `190 / 190` external replay)

More detail:

- [docs/benchmark-2026-04-18.md](docs/benchmark-2026-04-18.md)
- [docs/benchmark-methodology.md](docs/benchmark-methodology.md)
- [docs/benchmark-2026-04-10.md](docs/benchmark-2026-04-10.md)
- [docs/swebench-lite-plan.md](docs/swebench-lite-plan.md)
- [docs/court-jester-overview.md](docs/court-jester-overview.md)

## Development

Contributor commands are intentionally kept in [`justfile`](justfile):

```bash
just build
just test
just smoke
just smoke-sample
just fmt
just bench-dry-run
```

More repo and benchmark detail:

- [AGENTS.md](AGENTS.md)
- [docs/README.md](docs/README.md)
- [bench/README.md](bench/README.md)
