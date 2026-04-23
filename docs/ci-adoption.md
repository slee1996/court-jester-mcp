# CI Adoption Guide

This is the shortest path to running Court Jester in CI without drowning in noise or artifacts.

## Recommended Starting Point

For changed files in a PR, the shortest path is now the first-party wrapper:

```bash
court-jester ci \
  --base origin/main \
  --gate complexity,portability,execute \
  --report github \
  --report-level minimal
```

If you need the lower-level building blocks directly, the equivalent manual shape is:

```bash
court-jester verify \
  --file src/example.ts \
  --language typescript \
  --project-dir . \
  --diff-file /tmp/pr.diff \
  --report-level minimal \
  --execute-gate crash \
  --suppressions-file .court-jester-ignore.json \
  --output-dir .court-jester/reports
```

Why this shape:

- `court-jester ci` shells out to git, scopes to changed Python/TypeScript files, and aggregates the result into one CI-oriented exit code
- `--diff-file` limits work to changed functions
- `--report-level minimal` keeps artifacts small
- `--execute-gate crash` is the safest first gate for noisy repos
- `--suppressions-file` lets you keep CI on while tracking known findings explicitly
- auto-seeding stays on by default, so nearby tests can donate literal inputs without becoming an authoritative gate

## Suppression File Format

Court Jester accepts a JSON file with a `rules` array.

Example:

```json
{
  "rules": [
    {
      "path": "src/hotel-cache.ts",
      "stage": "execute",
      "function": "jsonResponse",
      "severity": "crash",
      "error_type": "RangeError"
    },
    {
      "path": "src/authz/check.ts",
      "stage": "complexity",
      "function": "checkRelation"
    },
    {
      "path": "src/main.ts",
      "stage": "portability",
      "reason": "err_module_not_found"
    }
  ]
}
```

Matching rules:

- `path` uses suffix matching against the verified source file path
- `stage` scopes the rule to `execute`, `complexity`, or `portability`
- `function`, `severity`, `error_type`, and `reason` narrow the match further

Suppressed findings still appear in the JSON report.

## Report Levels

Use `full` when:

- debugging a new integration
- investigating an unexpected failure
- auditing raw stderr or full parse output

Use `minimal` when:

- uploading artifacts from CI
- building dashboards
- annotating pull requests

If auto-seeding causes confusion during debugging, add `--no-auto-seed` to force Court Jester back to generator-only inputs.

## Cost Curve Guidance

Current field guidance from recent TypeScript adoption runs:

- median single-file verify time: about `5s`
- p95 single-file verify time: about `15s`
- a `10`-file parallel run: about `30s`
- a `30`-file PR at similar mix: about `90s`

The execute stage dominates most of the runtime. Coverage, portability, and report serialization are comparatively cheap.

Practical implications:

- parallelize at the file level
- use `--diff-file` whenever possible
- start with `--report-level minimal`
- treat authoritative `--test-file` usage as opt-in, because it adds real runtime cost

## Suggested Rollout

1. Start with `--execute-gate crash`.
2. Run with `--report-level minimal`.
3. Add suppressions for known false positives instead of turning off execute globally.
4. Only move to `--execute-gate all` after reviewing a few PRs worth of findings.
