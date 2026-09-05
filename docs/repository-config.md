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

Configuration is currently read from the working tree, even when CI uses `--head` to select a revision's diff. Committed-configuration selection remains unfinished; do not interpret a revision label as proof that configuration was loaded from that commit.

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

Use `doctor --show-config` to skip even the runtime/linter readiness probes. Configured-import and actual entrypoint-execution probes, as well as project-aware isolated doctor support, remain unfinished.

Replay does not read repository configuration: persisted replay context remains authoritative, with the existing explicit replay overrides. There is no configuration inheritance or executable configuration hook.
