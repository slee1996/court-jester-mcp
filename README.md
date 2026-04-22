# Court Jester

> **Public alpha / experimental**: Court Jester is under active development. CLI flags, output fields, and behavior may still change. The benchmark evidence is strong, but the tool is still early and should be treated as an alpha verifier, not a stable general-purpose coding product.

**Court Jester is a CLI for making AI-generated Python and TypeScript code fail as fast as possible before the agent declares victory.**

AI coding agents confidently ship code that looks right but quietly breaks on edge cases. They don't know what they don't know. Court Jester runs right after the edit, tries to break the changed file immediately, and turns "this looks done" into a concrete repro the agent can repair.

Today, the clearest way to think about Court Jester is: a strong alpha for Python and TypeScript repair loops, especially on library and utility code, not a polished universal answer for arbitrary repos.

```text
agent edits code -> court-jester verify -> fast concrete failure?
                                        |
                             yes: repair from repro
                             no:  ship with more confidence
```

It is just a CLI. No MCP transport, editor plugin, or custom agent integration layer is required.

Release and CI wiring docs:

- [CHANGELOG.md](CHANGELOG.md)
- [docs/report-schema.md](docs/report-schema.md)
- [docs/ci-adoption.md](docs/ci-adoption.md)
- [docs/proof-points.md](docs/proof-points.md)

## Why Use It

- Finds runtime and semantic failures, not just style issues
- Produces concrete repros instead of vague "something seems wrong" feedback
- Fits into any agent loop or CI job because it shells out like any other CLI
- Uses the target project's Ruff and Biome config instead of detached temp-dir defaults
- Returns structured JSON so automation can make decisions on pass/fail

## Where It Helps Most

- Library and utility code more than full application code
- Shared helpers, parsers, serializers, normalizers, validators, and cross-file semantic logic
- Python and TypeScript agent loops where you want to verify a changed file immediately after an edit
- Spec-like behavior where small semantic mistakes matter a lot
- Hidden semantic bugs that slip past obvious happy-path checks
- Nullish, fallback, defaulting, canonicalization, and cross-file behavior that looks plausible but is still wrong
- Repair loops where the model benefits from a concrete failing repro instead of generic feedback
- Projects that already have local tool context such as `.venv`, `node_modules`, Ruff, Biome, or authoritative test files

## Where It Is Not The Right Tool Yet

- Large app codebases where most value lives in integration glue, UI state, routing, auth flows, or framework wiring
- Product surfaces where end-to-end app behavior matters more than local file semantics
- Broad arbitrary-repo claims: the benchmark story is strong, but it is still an alpha, not a general guarantee on any repo
- Languages beyond Python and TypeScript
- Full CI replacement: Court Jester is a hostile verifier for agent loops, not a substitute for a real test suite
- Large framework or monolith construction tasks as a universal default workflow
- Security or secrecy-critical judging: hidden-eval benchmarking is a harness feature, not a hardened external judge

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
- prints a Biome follow-up when no sibling or `PATH` Biome is available for TypeScript lint

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
- TypeScript execute/verify: [Node.js](https://nodejs.org/) (Node 24+ recommended)
- Bun is only needed when the target repo is Bun-native and Court Jester falls back to the repo runtime for compatibility

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

TypeScript `--test-file` runs under Node. Test files that import `bun:test` are not currently supported as authoritative tests; use a Node-runnable test file instead, or omit `--test-file`.

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
| `coverage` | Reports exactly which functions were fuzzed, skipped, or blocked | No |
| `portability` | Preserves strict-Node portability issues separately from behavior | No |
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
- `--report-level full|minimal`: choose full debug output or CI-sized reports
- `--suppressions-file <PATH>`: JSON suppression rules for known findings
- `--no-auto-seed`: disable automatic seed extraction from nearby tests and simple literal call sites
- `--diff-file <PATH>`: only inspect changed functions from a unified diff
- `--complexity-metric cyclomatic|cognitive`: choose which complexity metric drives threshold failures
- `--complexity-threshold <N>`: fail when a function exceeds the threshold
- `--execute-gate all|crash|none`: choose which execute severities fail the run

Sandbox flags for `execute`:

- `--timeout-seconds <F>`
- `--memory-mb <N>`

Use `court-jester --help` for the full CLI help text.

## Exit Codes And Output

- `0`: command succeeded and the code passed
- `1`: the code failed verification or execution, but Court Jester still returned structured JSON
- `2`: CLI usage or setup error

Verify reports now carry `schema_version: 2` at the top level. The stability contract for stage names and JSON keys lives in [docs/report-schema.md](docs/report-schema.md).

That makes it easy to use in:

- agent loops
- pre-merge checks
- local shell workflows
- benchmark harnesses

## Evidence

Court Jester's strongest finished benchmark package is now the full causal-control package, not just the earlier one-arm verify-only rerun.

- One-repair causal matrix on `core-current`:
  - `baseline`: `208 / 234`
  - `public-repair-1`: `205 / 234`
  - `retry-once-no-verify`: `216 / 234`
  - `repair-loop-verify-only`: `230 / 234`
- Public-repair proving ground:
  - `baseline`: `11 / 36`
  - `public-repair-1`: `14 / 36`
  - `retry-once-no-verify`: `19 / 36`
  - `repair-loop-verify-only`: `25 / 36`
- Two-repair robustness on `core-current`:
  - `baseline`: `137 / 156`
  - `public-repair-2`: `140 / 156`
  - `retry-twice-no-verify`: `150 / 156`
  - `repair-loop-verify-only-2`: `156 / 156`
- False-positive gauntlet: `270 / 270` passes (`80 / 80` local known-good, `190 / 190` external replay)

More detail:

- [docs/benchmark-2026-04-20.md](docs/benchmark-2026-04-20.md)
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
