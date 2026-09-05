# Repository configuration

`verify`, `ci`, and `doctor` discover `.court-jester.json` or accept an explicit `--repo-config PATH`. Configuration is JSON, versioned, and rejects unknown fields. It is separate from `--config-path`, which selects Ruff/Biome configuration.

```json
{
  "schema_version": 1,
  "defaults": {
    "timeout_seconds": 10,
    "memory_mb": 256,
    "test_runner": "auto",
    "coverage_gate": "changed-exports",
    "test_files": ["tests/check_profile.py"]
  }
}
```

Place the file at your intended project root, for example `.court-jester.json`. Explicit selection is useful when the file has another name or lives outside the discovery path:

```sh
court-jester verify --repo-config .court-jester.json --file src/profile.py --language python
```

Supported `defaults` fields are `timeout_seconds`, `memory_mb`, `runtime_profile`, `test_runner`, `coverage_gate`, `execute_gate`, `inferred_oracle_gate`, `test_files`, and `suppressions_file`. Scalar options use the same values and validation as their corresponding CLI flags. Numeric budgets must be positive and within CLI limits. `test_files` is an array of strings; `suppressions_file` is a path string.

Precedence is built-in defaults, then repository defaults, then explicit CLI flags. An explicit `--test-file` list replaces the configured list rather than appending to it. Invalid scalar defaults are rejected even when overridden, so a typo does not remain hidden until the next invocation.

## Source-to-test mappings

Use top-level `targets` to select tests for exact source files:

```json
{
  "schema_version": 1,
  "targets": [
    {"source": "src/profile.py", "test_files": ["tests/check_profile.py"]},
    {"source": "src/pricing.py", "test_files": ["tests/check_pricing.py"]}
  ]
}
```

Mapping paths are relative to the config directory. Sources must exist and resolve to distinct files; aliases of the same source are rejected. Mappings require a nonempty test list. Matching uses resolved file identity, not globbing or declaration order. A matched target's test list replaces `defaults.test_files`; explicit CLI test files override both mappings and defaults. Unmatched sources use default tests, if configured. Direct verification retains its single-test-file limit, while CI selects at most one matching-language entrypoint for each source. CI's optional mutation budget remains global across all mapped targets.

Configured test and suppression paths are relative to the configuration file's directory. Unless explicitly overridden with `--project-dir`, that directory is also the dependency/runtime project root. CLI paths retain their ordinary invocation-directory semantics. CI still selects its Git repository from the invocation directory. Configured tests run as authoritative checks in CI without requiring `--test-quality`; mutation testing remains opt-in, with at most one test entrypoint per language.

By default, configuration and candidate files are read from the working tree, even when CI uses `--head` to select a revision's diff. A revision label alone does not identify the candidate's source state.

CI resolves base/head commit IDs once for its Git reads and includes `base_commit`, `head_commit`, and `candidate_state` in JSON reports. Committed workspaces read Git blobs directly: export-ignore/export-subst attributes, checkout filters, dirty files, and untracked files do not change initial snapshot bytes. Executable modes and internal symlinks are retained. Submodules and escaping, unresolved, or cyclic symlinks currently produce explicit materialization errors; dependencies are not installed automatically.

## Committed candidates

```sh
court-jester ci --base origin/main --head HEAD --candidate-state committed \
  --output-dir .court-jester/reports --report json
court-jester ci --head HEAD --candidate-state committed --show-config
```

Committed mode materializes the selected head before discovering repository configuration. Candidate source, imported tracked siblings, configured tests, suppression files, and configuration come from that workspace together. CLI input paths retain their invocation-directory meaning but are mapped into the selected commit; inputs outside the repository are rejected. Configured paths and source/test mappings must stay within the committed workspace. CLI scalar overrides still win. Missing committed files are errors, not fallbacks to dirty or untracked files.

Discovery starts at the snapshot counterpart of the invocation directory, walks to the snapshot root, and preserves the explicit-project-directory hard boundary. CI invoked from a nested directory still selects repository-relative changed source paths. `--no-repo-config` disables discovery in the selected state; `--repo-config` selects its committed counterpart. Use `--candidate-state working-tree` for ordinary checkout inputs, including external dependency roots.

Execution in committed mode requires `--output-dir`. A uniquely named snapshot is retained under its `candidate-workspaces/` directory so saved findings can replay after CI exits. JSON and human reports identify this workspace. Replay targets the retained source, not the original checkout; edit the retained candidate to test a repair, or explicitly select a different replay dependency context. Retention is not immutability: runtime code or later edits may change the snapshot. Archive or remove these generated artifacts when no longer needed, and keep the output directory out of version control. Existing snapshots are not reused or overwritten.

Persisted verification reports also receive unique filenames and are published without overwriting existing reports. Use the returned `report_path` rather than constructing a filename from the timestamp/source basename. This applies to ordinary verification as well as committed CI runs.

`--show-config` needs no output directory and does not retain its temporary workspace or run source/tests. Its paths describe that inspection snapshot and disappear afterward. Snapshot selection is not a hermetic dependency or environment guarantee: host runtimes, installed packages, and explicit runtime/environment settings still affect execution. Dependency installation and external project roots in committed mode are not supported automatically.

CI and direct verification both load the selected suppression file, reject missing files or invalid rules, and pass its rules and source path to verification. CI validates the file even when no changed sources are selected. Suppression matching and retained suppressed evidence use the same verifier rules in both commands.

Suppression files are JSON objects with an optional `rules` array. Each rule accepts only `path`, `stage`, `function`, `severity`, `error_type`, and `reason`. Selectors are combined; strings must be nonempty and each rule needs at least one selector. `stage` is `execute`, `complexity`, or `portability`; severity is `crash`, `property_violation`, `behavioral_regression`, or `infrastructure`. Unknown fields and invalid types are errors, not ignored rules. `{}` and `{"rules":[]}` explicitly select no suppressions; an empty rule `{}` is rejected. A deliberately broad rule can still name a stage. `reason` matches the finding's reason; it is not a free-form justification field. Library verification also rejects invalid suppression data with an inconclusive configuration report before executing source or tests.

Discovery starts at `--project-dir` when supplied and checks only that directory. Otherwise `verify` and doctor with `--file` start at the target file's directory; CI and doctor without a target start at the invocation directory. Within a Git repository, discovery walks up to the nearest Git root, inclusively, and chooses the nearest configuration without merging parent files. Without a Git root, it checks only the starting directory. It never searches above the Git root. Git worktree `.git` files are also boundaries.

Use `--no-repo-config` to bypass discovery, including a malformed discovered config. It conflicts with `--repo-config`. Invalid discovered files produce errors rather than silently falling back to another config.

## Inspect selected settings

```sh
court-jester verify --file src/profile.py --language python --show-config
court-jester ci --show-config
```

`--show-config` prints JSON without running source code, tests, native engines, or readiness probes. It reports the selected configuration path, whether discovery was disabled, project override, selected tests/mappings, policies, and ordinary verification limits. Source and language may be omitted to inspect repository-wide settings. Timeout values use the verifier's actual resolver, including environment defaults and configured/CLI overrides; specialized adapters can still have different minimum budgets. Project overrides are not a claim that runtime resolution or imports succeeded. `execution_started` and `readiness_checked` are both false.

## Doctor

Doctor resolves the same defaults and source/test mappings. Its `repository_config` check shows the selected verification settings, while `configured_entrypoints` checks that selected tests are readable regular files. Missing tests fail with guidance to restore the file or update `test_files`. These checks do not import the target or run the tests. Runtime smoke probes use configured memory when supplied, otherwise their existing 128 MB default; the displayed ordinary verification memory default remains 512 MB.

Use `doctor --show-config` to skip even the runtime/linter readiness probes. It remains execution-free when `--probe-entrypoint` is also present.

### Opt-in entrypoint readiness

```sh
court-jester doctor --file src/profile.py --language python --probe-entrypoint
court-jester doctor --file src/profile.ts --language typescript \
  --test-file tests/profile.test.ts --test-runner node --probe-entrypoint
```

`--probe-entrypoint` explicitly runs the selected authoritative test entrypoint and its imports. It requires one source file, one language, and exactly one configured or explicit test file. Without this flag, doctor does not run target imports or tests: `project_context` only validates paths, and `configured_entrypoints` only checks readability.

The probe supports both runtime profiles. `local-trusted` executes trusted project code on the host. Add `--runtime-profile isolated` to use verification's existing Docker execution path, selected image overrides, no network, read-only project mounts, and resource limits. Images and dependencies must already be available; doctor neither installs dependencies nor pulls images and never falls back to host execution. Image smoke checks alone do not establish project readiness; the opt-in entrypoint must also pass. `just isolated-onboarding-check` runs real Python/Node container regressions, including concurrent Node mutation campaigns, using installed images.

The probe uses verification's normal test adapter and context resolution, but runs neither fuzzing nor mutation tests. It requires successful test execution and per-run evidence that the selected target module finished loading. Exit zero without that evidence is inconclusive in the nested test stage and fails the doctor readiness check. A passed probe does not prove function coverage or application correctness. Broken imports, assertions, unavailable adapters, resource limits, and instrumentation failures remain visible in `entrypoint_probe.detail.test_stage`; read errors appear in `error`.

Timeout defaults to doctor's 10 seconds per probe. Entrypoint memory defaults to ordinary verification's 512 MB, while lightweight runtime smoke remains 128 MB. Explicit/configured limits override both. For a timeout or memory failure, inspect the structured execution details and repair the cause or explicitly adjust `--timeout-seconds`/`--memory-mb`. For missing imports, restore the dependency or repair project/runtime selection before retrying.

Both runtime profiles require successful smoke execution plus exactly one structured version/executable record, with Python 3 or Node.js >=24. A zero exit code or unstructured output alone is not readiness. Missing, malformed, or duplicate records fail the check. Isolated `runtime_smoke` retains the structured execution result and detected runtime details alongside raw stdout/stderr; image inspection alone does not prove the runtime executed.

Docker daemon and image readiness commands use the configured per-probe timeout and memory budget (10 seconds/128 MB by default), with process-group cleanup on timeout. Failed daemon/image readiness skips dependent runtime checks; failed isolated readiness also skips a requested entrypoint instead of trying it anyway.

For each isolated harness launch, Docker image inspection, create, start, wait, state inspection, and log collection share the remaining launch deadline. Cleanup has a separate allowance of at most five seconds (or the smaller configured timeout), so deadline exhaustion still permits a removal attempt. A failed cleanup is a blocker and reports the container name for inspection; the tool cannot guarantee removal while Docker is unavailable. These bounds cover managed CLI/process work, including inherited output pipes, not arbitrary filesystem stalls during host project preparation. Doctor can run multiple probes, so its total duration can exceed a single probe budget.

Ctrl-C or SIGTERM cancels the command and waits up to 20 seconds for owned container workers to finish cleanup, then exits 130 or 143. Create/start calls have a ten-second control-plane ceiling so they can settle before removal; cleanup retains tool-owned temporary resources. Hard termination (such as SIGKILL) or an unavailable Docker daemon cannot guarantee removal. Inspect any container named in an unconfirmed-cleanup diagnostic before continuing.

`just onboarding-check` runs temporary Python and TypeScript project acceptance sequences: inspect settings, check default readiness without entrypoint execution, detect a missing import, repair it, then detect a failing test. Both successful and deliberately failing entrypoints must have target-load evidence; a fresh marker proves the failing phase actually ran. TypeScript uses an ESM project and Node's test runner and requires Node.js >=24 on PATH. It installs nothing and records separate binary-bound evidence per language; it is not participant research or proof that every framework is supported. The quality/release gate runs both sequences and preserves their artifacts. Run either directly with `python3 scripts/check_onboarding.py --language python` or `--language typescript`; Python remains the script's default.

Replay does not read repository configuration: persisted replay context remains authoritative, with the existing explicit replay overrides. There is no configuration inheritance or executable configuration hook.
