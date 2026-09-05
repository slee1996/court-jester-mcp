# Court Jester

> **Public alpha / experimental**: Court Jester is under active development. CLI flags, output fields, and behavior may still change. The benchmark evidence is strong, but the tool is still early and should be treated as an alpha verifier, not a stable general-purpose coding product.

**Court Jester is a CLI for making AI-generated Python and TypeScript code fail as fast as possible before the agent declares victory.**

AI coding agents confidently ship code that looks right but quietly breaks on edge cases. They don't know what they don't know. Court Jester runs right after the edit, tries to break the changed file immediately, and turns "this looks done" into a concrete repro the agent can repair.

Today, the clearest way to think about Court Jester is: a strong alpha for Python and TypeScript repair loops, especially on library and utility code, not a polished universal answer for arbitrary repos.

```text
agent edits code -> court-jester verify
                     fail: repair from repro, then reverify
             inconclusive: resolve missing evidence or environment
                     pass: continue repository tests and review
```

It is just a CLI. No MCP transport, editor plugin, or custom agent integration layer is required.

Release and CI wiring docs:

- [docs/code-map.md](docs/code-map.md)
- [CHANGELOG.md](CHANGELOG.md)
- [docs/releasing.md](docs/releasing.md)
- [docs/report-schema.md](docs/report-schema.md)
- [docs/ci-adoption.md](docs/ci-adoption.md)
- [docs/proof-points.md](docs/proof-points.md)

## Why Use It

- Finds runtime and semantic failures, not just style issues
- Produces concrete repros instead of vague "something seems wrong" feedback
- Fits into any agent loop or CI job because it shells out like any other CLI
- Uses the target project's Ruff and Biome config instead of detached temp-dir defaults
- Returns typed JSON (`verdict`: `pass`, `fail`, or `inconclusive`) plus evidence `strength` so automation can distinguish a clean result from a coverage or environment gap

## Where It Helps Most

- Library and utility code more than full application code
- Shared helpers, parsers, serializers, normalizers, validators, and cross-file semantic logic
- Python and TypeScript agent loops where you want to verify a changed file immediately after an edit
- Spec-like behavior where small semantic mistakes matter a lot
- Hidden semantic bugs that slip past obvious happy-path checks
- Nullish, fallback, defaulting, canonicalization, and cross-file behavior that looks plausible but is still wrong
- Exported object/class methods, factory-returned callables, and supported container patterns such as Zustand-style `create(... => ({ ... }))` stores
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
Repair when `verdict: fail`; inspect the report and add a contract or authoritative test when `verdict: inconclusive`.
After `verdict: pass`, continue the repository's normal tests and review. A pass is scoped evidence, not proof that the change is correct or ready to ship.
Treat persisted findings and their replay commands as the repair contract.
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

TypeScript with an authoritative test file and the stable advisory behavior-sensitivity check:

```bash
court-jester verify \
  --file src/semver.ts \
  --language typescript \
  --project-dir . \
  --test-file tests/semver.test.ts \
  --test-quality 8
```

Direct `verify` requires exactly one `--test-file` when `--test-quality [N]` is enabled. Omitting `N` uses 8 mutants; explicit budgets must be from 1 through 32. `--tests-only` is optional: without it, the normal verification pipeline runs before the advisory `test_quality` stage.

TypeScript `--test-file` uses `--test-runner auto` by default. That path prefers Bun when the authoritative test imports `bun:test`; otherwise it uses the Node path. Use `--test-runner node|bun|repo-native` to override.

The bounded campaign uses conservative comparison-boundary, equality, condition-negation, and boolean mutants. A mutant is judged only after the authoritative test enters its exact mutated public surface:

- `killed`: the entered mutant made the authoritative test fail;
- `survived`: the entered mutant left the authoritative test passing, producing a concrete missing-observation witness;
- `invalid`: the mutation could not be applied or validated;
- `blocked`: runner, instrumentation, sandbox, timeout, memory, or other infrastructure prevented judgment;
- `no_coverage`: the authoritative test did not enter the exact mutated surface.

Only `killed` and `survived` are judged outcomes. `invalid`, `blocked`, and `no_coverage` are explicit unjudged evidence, never failed kills. Target-aware private access/import/spy and source-introspection findings are reported separately because mutation sensitivity cannot prove implementation independence; each finding names its normalized authoritative `test_source_file`.

The feature is advisory: it never changes `verdict`, `strength`, process exit status, or CI gates. It emits no score, percentage, grade, or threshold.


Source directives:

- `court-jester-ignore complexity` on the same line or immediately above a callable suppresses only that complexity finding while keeping it visible in the report.
- `court-jester-properties ...` on the same line or immediately above a callable adds explicit execute-stage properties without renaming the function.

Example:

```ts
// court-jester-properties sorted permutation
export function reorder(values: string[]): string[] {
  return [...values].reverse();
}
```

Supported declarative properties today:

- `idempotent`
- `sorted`
- `permutation`
- `nonnegative`
- `clamped`
- `nonempty_string`
- `symmetric`
- `antisymmetric`
- `bounded`
- `no_nullish_string`

Precedence:

- explicit properties are additive: they turn checks on even when the function name would not have triggered a built-in heuristic
- built-in heuristics still run unless separately suppressed
- `antisymmetric` maps to the existing comparator-style contract checks

Write JSON reports to disk:

```bash
court-jester verify \
  --file src/profile.py \
  --language python \
  --output-dir .court-jester/reports
```

Other commands:

```text
court-jester ci       [OPTIONS]
court-jester analyze  [OPTIONS]
court-jester lint     [OPTIONS]
court-jester execute  [OPTIONS]
court-jester --help
```

Changed-files CI wrapper with explicit, repeatable language entrypoints:

```bash
court-jester ci \
  --base origin/main \
  --head HEAD \
  --project-dir . \
  --test-file tests/test_all.py \
  --test-file tests/all.test.ts \
  --test-quality 8 \
  --report github \
  --report-level minimal
```

In `ci`, `--test-file` is repeatable with at most one Python entrypoint and at most one TypeScript/TSX entrypoint. Court Jester selects the matching entrypoint for each changed target by language; it does not infer source-to-test mappings. The `--test-quality N` cap is global for the entire CI command, not per file. Candidate allocation is deterministic across changed files and their required public surfaces, never exceeds `N`, and may explicitly underfill when candidates or matching tests are unavailable. `--tests-only` remains unsupported in CI. Per-file and aggregate output reports planned, killed, survived, unjudged, and coupling counts without a score.

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
| `test_quality` | Optional bounded mutation sensitivity and target-aware coupling evidence | No, advisory only |

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

- `--test-file <PATH>`: add exactly one authoritative test stage
- `--test-runner auto|node|bun|repo-native`: choose how TypeScript authoritative tests execute
- `--tests-only`: skip fuzz execute and run only the authoritative test stage
- `--test-quality [N]`: run a stable advisory behavior-sensitivity campaign (default 8; range 1..32); requires `--test-file`
- `--output-dir <PATH>`: persist JSON reports
- `--report-level full|minimal`: choose full debug output or CI-sized reports
- `--suppressions-file <PATH>`: JSON suppression rules for known findings
- `--no-auto-seed`: disable automatic seed extraction from nearby tests and simple literal call sites
- `--native-fuzz-engine off|auto|atheris|jazzer` with `--native-fuzz-runs <N>`: opt into an installed coverage-guided engine after the deterministic campaign. `auto` selects Atheris for Python or Jazzer.js for TypeScript and reports unavailable engines explicitly instead of silently falling back.
- `--llm-plateau-command <PATH>`: opt into an external JSON seed proposer only after the retained deterministic corpus stops growing; accepted seeds still run through the normal sandbox, finding, minimization, and replay pipeline.
- `--diff-file <PATH>`: only inspect changed functions from a unified diff
- `--complexity-metric cyclomatic|cognitive`: choose which complexity metric drives threshold failures
- `--complexity-threshold <N>`: fail when a function exceeds the threshold
- `--execute-gate all|crash|none`: choose which execute severities fail the run
- `--base-file <PATH>` with `--base-project-dir <PATH>`: compare the candidate against a complete read-only baseline tree; both options are required together.
- `--summary json|human|repair-json`: select machine, human, or repair-loop output.

Runtime and evidence controls:

- `--coverage-gate changed-exports|none` (default `changed-exports`) requires changed exported/invocable surfaces to be behaviorally checked; `none` disables per-surface enforcement but does not turn zero evidence into a pass.
- `--inferred-oracle-gate advisory|fail` (default `advisory`) keeps low-confidence name/context findings non-gating unless explicitly promoted.
- `--runtime-profile local-trusted|isolated` (default `local-trusted`) selects host execution or Docker isolation. Isolated mode accepts `--python-docker-image` (`python:3.12-slim`) and `--typescript-docker-image` (`node:24-bookworm-slim`).
- `court-jester doctor --language python|typescript|all` checks the selected runtime; use its schema-v3 report as a prerequisite for benchmark evidence.
- For local project readiness, run `court-jester doctor --language python --project-dir . --file src/example.py`. It uses the shared project/runtime resolver and linter precedence, runs a small generated runtime probe, and reports the selected executable and version. `--file` is optional with `--project-dir`; a file requires one explicit language. `--timeout-seconds` bounds each runtime/linter probe. This does not import the target, validate its dependencies, or run its tests; a readiness pass is not target verification. Isolated doctor still checks image readiness only and rejects project/file options.
- `court-jester replay --report <PATH> --finding <ID>` reruns a persisted structured repro and returns a typed replay outcome.
- `just bench-fuzz-effectiveness` runs the seeded mutation/control lane and reports mutation recall plus clean-control specificity. The command exits nonzero on any expected-finding, stage, verdict, or specificity mismatch.

Sandbox flags for `execute`:

- `--timeout-seconds <F>`
- `--memory-mb <N>`

Use `court-jester --help` for the full CLI help text.

## Exit Codes And Output

- `0`: the command produced a passing verdict (or a replay reproduced its expectation)
- `1`: verification failed (or replay did not reproduce)
- `2`: CLI usage, argument, or setup error before a report
- `3`: verification was inconclusive (coverage, runtime, timeout, or other missing evidence)

Verify and persisted reports carry `schema_version: 3` at the top level. Reports expose typed `verdict` and `strength`; stages use `status` values such as `passed`, `failed`, `inconclusive`, `advisory`, and `skipped`. The stability contract for keys and findings lives in [docs/report-schema.md](docs/report-schema.md).

The intended repair view is `court-jester verify ... --summary repair-json`: it returns `recommended_action` (`repair`, `inspect_environment`, `add_contract_or_test`, or `none`) without changing the exit code.
Benchmark evidence is a separate contract: when matrix, run, result, summary, or evidence-manifest files are generated or supplied, they use `artifact_schema_version: 1` and require verifier schema `3`. Missing or mixed versions are abstentions (or hard errors in strict evidence mode), not successful runs. This format contract does not imply that a test-quality-specific matrix, oracle, result set, or evidence bundle is committed or gates the advisory feature. The general benchmark harness supports `--verify-runtime-profile local-trusted|isolated`, `--doctor-report`, `--gate-policy none|private-beta-default|strict-heldout`, `--evidence-bundle`, and opt-in `--shadow-records`; shadow records never change run success.

The repository's working install and source URLs intentionally retain the historical `court-jester-mcp` GitHub path until a confirmed remote rename; do not substitute an unverified URL.

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
