# Court Jester 0.2.8

Release date: 2026-08-15

Court Jester 0.2.8 replaces the synthetic standalone assumptions behind the reopened TypeScript runner defect with explicit project-native adapters. Complex Vitest, Nuxt, pnpm-workspace, and imported-type surfaces now execute through the repository environment that owns their semantics, while standalone Python and TypeScript verification retain their existing direct path. This release supersedes the unpublished 0.2.7 candidate after Linux Node 24 exposed a native module-loading instrumentation gap.

## Project-native verification

- A reported `project_adapter` contract records the selected adapter, workspace/package roots, dependency roots, runner, capabilities, and rationale.
- Every exported surface receives an explicit execution strategy and expected evidence before execution.
- Existing Vitest tests run from their original workspace path. Court Jester no longer rewrites the target source or generated framework files to collect reachability evidence.
- In-memory target instrumentation registers Node's synchronous module-load hook on Node 24+ and a guarded asynchronous hook on older supported Node versions. Native TypeScript loading therefore receives the instrumented source while preserving all repository files. Docker runs receive the same preload and payload through read-only mounts.
- The portable Vitest coordinator resolves workspace packages, TypeScript path aliases, Nuxt aliases, and extensionless project modules; it retains one-worker bounds plus network and process guards.
- When a repository config imports a host-platform native package that cannot run in the isolated Linux image, the coordinator reports the incompatibility and uses its portable transform rather than falling back to plain Node/Bun execution.

## Synthesis and runtime repairs

- Imported aliases, readonly arrays, constructor-like interfaces, schema-inferred object shapes, default initializers, and Python `Protocol` inputs now produce executable candidates.
- Decorated TypeScript exports are instrumented without inserting code between a decorator and its declaration.
- pnpm dependency paths are materialized inside the configured workspace boundary instead of escaping through symlink/store paths.
- Python harnesses import targets under a non-main module identity so guarded CLI/server entry points do not execute accidentally.
- Ruff processes terminated by the host are reported as environment failures with the exact invocation, not as target lint violations.

## Evidence model

- Authoritative tests credit only exact `target_entered` surface IDs emitted during a successful project-runner invocation.
- Static analysis, generated execution, authoritative tests, and portability remain orthogonal in the new `outcome_matrix`; a complexity finding cannot erase successful behavioral evidence.
- Full and minimal reports retain the adapter and surface execution plan needed to distinguish target failures from project-runtime or capability failures.

## Red/green evidence

Focused regressions cover original-path Vitest launch, native module-load instrumentation without source mutation, Nuxt adapter selection, pnpm workspace resolution, decorator-safe instrumentation, imported-alias synthesis, Python guarded imports, and exact authoritative surface credit.

Real-project controls were also exercised against the Resin8 reproduction workspace:

- `extractPdfTableCells.ts` passed its native authoritative Vitest suite in the isolated profile with 2/2 required exported surfaces credited.
- `useConfigurableProductsFeatureFlag.ts` passed its Nuxt composable tests through the portable isolated Vitest coordinator.
- The bundled standalone Python sample passed with property-checked strength and 1/1 required surface behaviorally checked.

## Release validation

The 0.2.8 release candidate passed the complete `release-check` recipe: release metadata, formatting, Clippy with warnings denied, 416 locked Rust integration tests, 69 benchmark unit tests, 2 release-contract tests, optimized build, CLI/verification smoke, package staging, and the held-out matrix dry run. The release workflow repeats those gates on Linux Node 24 before publishing platform archives.
