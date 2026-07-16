# Court Jester 0.2.0

Release date: 2026-07-15

Court Jester 0.2.0 is the confidence-contract release. It replaces ambiguous boolean success with explicit evidence accounting: a verification now ends in `pass`, `fail`, or `inconclusive`, and the report explains the strength and coverage behind that verdict.

This is a breaking report-contract change for automation built against the 0.1.x series.

## Highlights

### Confidence-calibrated report schema v3

- The top-level report contract is now schema `3` with a typed `verdict`: `pass`, `fail`, or `inconclusive`.
- Reports expose evidence `strength`, typed stage statuses, coverage summaries, provenance-rich findings, structured repros, and stable invocation paths.
- A coverage gap, rejected-only input set, runtime setup failure, timeout, or missing behavioral evidence is `inconclusive` instead of a silent pass.
- `--summary repair-json` gives agent loops a compact `recommended_action`: repair, inspect the environment, add a contract or test, or continue.

### Strict surface and oracle accounting

- `--coverage-gate changed-exports` is the default and requires changed exported or otherwise invocable surfaces to be behaviorally checked.
- Coverage identifies direct, factory, caller, and authoritative-test invocation paths rather than treating discovery as proof of execution.
- Verification plans record surfaces, parameter domains, execution units, inputs, and contracts so a green result can be audited.
- Low-confidence name/context inferences and unproven differential findings remain advisory by default. Use `--inferred-oracle-gate fail` only when that policy is intentional.

### Replayable findings and differential verification

- Persisted findings include typed severity, confidence, oracle kind and provenance, source location, input classification, structured repro data, and minimization status.
- `court-jester replay --report <path> --finding <id>` replays a persisted finding without depending on the original temporary harness.
- `--base-file` plus `--base-project-dir` compares a candidate against a complete read-only baseline tree while keeping unproven differences advisory.
- Replayable comparator, return-contract, crash, query, semver, PEP 440, cookie, request/response, and declared-property cases gained broader regression coverage.

### Runtime readiness and isolation

- `court-jester doctor` checks Python, TypeScript, linter, Docker, and selected runtime prerequisites before a verification or benchmark lane.
- `--runtime-profile local-trusted|isolated` selects host execution or Docker isolation.
- Isolated mode supports explicit Python and TypeScript image overrides and records the chosen runtime identity in evidence.
- Local execution retains project-aware Ruff, Biome, Node, and Bun resolution, including Bun-backed authoritative tests and repo-native fallbacks.

### CI and benchmark evidence

- `court-jester ci` can gate changed files on parse, complexity, coverage, portability, execute, and authoritative-test stages with human, GitHub, or JSON output.
- The reusable quality workflow now runs formatting, Clippy, Rust integration tests, benchmark unit tests, release-contract tests, a locked release build, CLI smoke, and a locked held-out dry-run.
- Benchmark matrices, runs, results, summaries, and evidence manifests use artifact schema `1` and require verifier schema `3`.
- Benchmark summaries retain abstentions, separate operational errors from semantic outcomes, pair cells by hidden-seed digest, and emit paired lift, confidence intervals, and exact McNemar results when eligible.
- Evidence bundles are relative, checksummed, redaction-aware, and optionally strict. Shadow records remain non-blocking by contract.

### GitHub release integrity

- Official archives are built for macOS and Linux on Arm64 and AMD64.
- Every archive ships with a matching SHA-256 file; the release workflow verifies all checksums before publication.
- `install.sh` downloads and verifies the checksum and refuses an unverified archive.
- Public archives contain the `court-jester` binary only. Ruff and Biome remain project or operator dependencies.

## Breaking changes and migration

1. Require `schema_version: 3` in report consumers.
2. Branch on `verdict`; do not reconstruct a result from stage names and do not read the removed legacy `overall_ok` boolean.
3. Treat `inconclusive` as missing evidence, not a pass and not a confirmed semantic failure.
4. Update exit-code handling:
   - `0`: passing verdict or successful replay
   - `1`: failed verification or replay mismatch
   - `2`: CLI usage or setup failure before a report
   - `3`: inconclusive verification
5. Update agent prompts to repair `fail`, investigate the environment or add coverage for `inconclusive`, and ship only `pass`.
6. Regenerate benchmark artifacts under artifact schema `1`; do not mix legacy artifacts into an active release gate.

Historical schema-v2 reports and pre-artifact-v1 benchmark outputs remain historical evidence. Court Jester does not silently rewrite them into the new contracts.

## Install

Court Jester currently ships through GitHub Releases:

```bash
curl -fsSL https://raw.githubusercontent.com/slee1996/court-jester-mcp/main/install.sh | sh
```

The installer selects the current macOS/Linux architecture, verifies the published checksum, and installs `court-jester` to `~/.local/bin`.

## Validation

The final release candidate passed:

- `cargo fmt --all -- --check`
- `cargo clippy --locked --all-targets -- -D warnings`
- 328 Rust unit and integration tests
- 65 benchmark-harness unit tests
- 2 release-contract tests, including mismatched-tag rejection
- locked optimized build and CLI smoke against `court-jester 0.2.0`
- real verify smoke against the bundled failing fixture
- GitHub archive staging with optional linters excluded
- locked `swebench-lite-pilot` benchmark dry-run with held-out-lock enforcement

The tag-driven GitHub workflow repeats the maintained quality gate before building and publishing platform assets.

## Known boundaries

- Python and TypeScript are the supported source languages.
- `local-trusted` executes target code on the host. Use `isolated` for untrusted target code when Docker is available.
- Ruff, Biome, Node.js, and optionally Bun are resolved from the target project or operator environment and are not bundled.
- Low-confidence inferred semantics remain advisory unless explicitly promoted.
- Differential verification is evidence, not an authoritative oracle, unless paired with a declared contract or authoritative test.
- Court Jester remains an experimental verifier for agent repair loops, not a general CI or secure hidden-judge replacement.
