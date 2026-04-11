# Court Jester

**An agent-focused CLI that catches real Python and TypeScript bugs before the agent declares success.**

AI coding agents are good at writing plausible code and bad at knowing when they are actually done. Court Jester sits inside that loop: the agent edits a file, Court Jester fuzzes it, and if it finds a concrete failure the agent gets a repro to fix before it stops.

```text
Agent edits code -> court-jester verify -> bug found?
                                         |
                              yes: agent repairs with repro
                              no:  agent can ship
```

## Benchmarked Results

The current strongest benchmark is a strict verify-only repair policy on the 39-task `core-current` suite.

One repair round was allowed only after a failed `court-jester verify`. Public and hidden evaluator failures did not trigger repair feedback.

| Model | Baseline | Verify-Only Repair Loop | Tasks saved |
|-------|----------|-------------------------|-------------|
| Claude | 35 / 39 (90%) | 37 / 39 (95%) | +2 |
| Codex | 36 / 39 (92%) | 39 / 39 (100%) | +3 |

Across both models, that is `71 / 78` baseline vs `76 / 78` with Court Jester.

Within the repair-loop arm itself:

- `11` runs triggered a repair round because `verify` failed
- all `11` trigger sources were `verify`
- `0` repair rounds were triggered by public or hidden evaluator feedback
- `10` of those verify-triggered repairs ended in final success

The known-good false-positive control still passes `20 / 20`.

## Get Started

### 1. Install

```bash
curl -fsSL https://raw.githubusercontent.com/slee1996/court-jester-mcp/main/install.sh | sh
```

This downloads `court-jester` into `~/.local/bin`. No Rust toolchain and no agent transport configuration are required.

<details>
<summary>Build from source instead</summary>

```bash
cargo install --git https://github.com/slee1996/court-jester-mcp.git
```

Use a current stable Rust toolchain.
</details>

### 2. Install the companion tools you actually want

Recommended:

- [ruff](https://docs.astral.sh/ruff/installation/) for Python lint
- [Biome](https://biomejs.dev/guides/getting-started/) for TypeScript lint
- [bun](https://bun.sh) for TypeScript fuzz execution

Court Jester resolves linters in this order:

1. Project-local binaries such as `.venv/bin/ruff`, `venv/bin/ruff`, and `node_modules/.bin/biome`
2. Optional sibling binaries next to `court-jester`
3. `PATH`

Public release assets should normally ship `court-jester` alone. Bundled linters are for controlled/local use only.

### 3. Tell your agent how to use it

Add this to your prompt or `AGENTS.md`:

```text
After every code change, run `court-jester verify --file <changed-file> --language <python|typescript>`.
If verify returns overall_ok: false, fix the failing repro and verify again.
Treat verify repros as authoritative.
```

That is the whole integration. The agent just shells out to the CLI.

## Quick Start Without An Agent

```bash
court-jester verify \
  --file bench/repos/mini_py_service/profile.py \
  --language python \
  --project-dir bench/repos/mini_py_service
```

This sample is expected to fail. The point is to show the execute stage finding a real runtime bug automatically.

## What `verify` Does

`verify` runs a staged pipeline and returns one JSON report:

| Stage | What it does | Fails the run? |
|-------|--------------|----------------|
| `parse` | Tree-sitter AST extraction | Yes |
| `complexity` | Optional cyclomatic complexity gate | Only if you set a threshold |
| `lint` | Ruff or Biome in the project context | No, advisory only |
| `execute` | Synthesized fuzz/property checks in a sandbox | Yes |
| `test` | Optional caller-supplied test file | Yes |

The important stage is `execute`. Court Jester walks the AST, resolves types across local imports, generates adversarial inputs, and runs those calls in a sandbox with time and memory limits. When something breaks, it returns the concrete repro.

For Python, common built-in runtime and validation exceptions such as `TypeError`, `AttributeError`, `KeyError`, `IndexError`, `ValueError`, `ZeroDivisionError`, and `UnicodeError` are treated as crashes, not harmless validation rejects.

## CLI Usage

```text
court-jester verify   [OPTIONS]
court-jester analyze  [OPTIONS]
court-jester lint     [OPTIONS]
court-jester execute  [OPTIONS]
court-jester --help
court-jester --version
```

Core flags:

- `--file <PATH>`: source file to inspect
- `--language python|typescript`
- `--project-dir <PATH>`: root for `.venv`, `node_modules`, and config discovery
- `--config-path <PATH>`: explicit Ruff/Biome config
- `--virtual-file-path <PATH>`: preserve lint path semantics for temp or generated files

Verify-specific flags:

- `--test-file <PATH>`
- `--output-dir <PATH>`
- `--diff-file <PATH>`
- `--complexity-threshold <N>`

Execute-specific flags:

- `--timeout-seconds <F>`
- `--memory-mb <N>`

Examples:

```bash
court-jester verify --file src/profile.py --language python
court-jester verify --file src/semver.ts --language typescript --test-file tests/semver.test.ts
court-jester lint --file src/parser.py --language python --config-path pyproject.toml
court-jester analyze --file src/router.ts --language typescript --diff-file changes.diff
court-jester execute --file src/demo.py --language python --timeout-seconds 5
```

Exit codes:

- `0`: command succeeded and the code passed
- `1`: the code failed verification or execution, but the CLI still returned structured JSON
- `2`: CLI usage or setup error

## Bring Your Own Lint Rules

Court Jester now runs linters in the user project context rather than a detached temp directory.

Use these patterns:

```bash
court-jester lint --file src/app.py --language python --project-dir .
court-jester verify --file src/app.ts --language typescript --project-dir apps/web
court-jester lint --file src/app.py --language python --config-path pyproject.toml
```

That means repo-local `pyproject.toml`, `ruff.toml`, `biome.json`, path-based overrides, include/exclude rules, and project-local linter binaries all work the way users expect.

## Release And Dev

Build and test:

```bash
cargo fmt
cargo test
cargo build --release
```

CLI smoke tests:

```bash
python scripts/smoke_cli.py --release
python scripts/smoke_cli.py --release --verify-sample
```

Stage a release directory:

```bash
python scripts/prepare_release.py --release
```

Optional local-only bundling:

```bash
python scripts/prepare_release.py --release --include-ruff --include-biome
```

Benchmark harness:

```bash
python -m bench.run_matrix --dry-run
python -m bench.run_matrix \
  --task-set core-current \
  --models codex-default,claude-default \
  --policies baseline,repair-loop-verify-only \
  --output-dir /tmp/court-jester-core-verify-only
python -m bench.summarize_runs bench/results/dev
```

## Repo Layout

```text
src/        Rust CLI and verification pipeline
tests/      Rust integration tests
bench/      Benchmark harness, task fixtures, evaluators, stress runs
docs/       Product notes, architecture docs, benchmark writeups
scripts/    smoke_cli.py, prepare_release.py
```

## Troubleshooting

| Problem | What it means |
|---------|---------------|
| `ruff not available in project, on PATH, or next to court-jester` | Install Ruff or point the CLI at the right project root |
| `biome not available...` | Install Biome or make sure `node_modules/.bin/biome` exists |
| TypeScript verify fails before fuzzing starts | Install `bun` or make sure the local TS runtime is available |
| Lint crashes but verify still reports code failures separately | Expected; lint is advisory and no longer blocks the full verify verdict by itself |

## More Detail

- [docs/court-jester-overview.md](docs/court-jester-overview.md)
- [docs/benchmark-2026-04-10.md](docs/benchmark-2026-04-10.md)
- [docs/system-flow.md](docs/system-flow.md)
- [docs/tool-flow-diagram.md](docs/tool-flow-diagram.md)
- [bench/README.md](bench/README.md)
