# Court Jester 0.2.12

Release date: 2026-08-15

Court Jester 0.2.12 fixes the shared isolated dependency boundary behind issues #30 and #31. Project-native Vitest verification now keeps one package-owned dependency graph from selection through execution, falls back only for positively identified native or mixed-runner failures, and retries transient dependency reads outside the TypeScript transform loader.

## Project-native Vitest correctness

- The portable coordinator resolves `@vitest/runner` from the selected Vitest package before considering a sibling fallback, preventing incompatible workspace-root and leaf-package versions from mixing.
- Project configuration is removed only after a recognized native toolchain failure. A second config-free attempt reaches the matching package runner only for recognized native or Vitest-internal initialization signatures; target syntax failures, unavailable environments such as `jsdom`, and other ordinary collection failures remain authoritative failures.
- Direct-runner summaries include file and nested-suite lifecycle failures even when no assertion body fails. A suite-level `beforeAll` failure can no longer become a passing report.
- The direct runner refuses non-empty Vitest filters it cannot preserve instead of broadening the authoritative test selection.
- JavaScript instrumentation preserves the in-memory payload without unnecessary transpilation. TypeScript, TSX, and JSX-family payloads are transpiled through the selected project TypeScript package.

## Isolated dependency boundary

- Docker dependency materialization uses one coherent workspace snapshot with package-first resolver roots and package-local transitive dependencies.
- The retrying package loader wraps the TypeScript loader in Node's last-in-first-out loader chain, so transient `EACCES` failures during TypeScript, JSX, JavaScript, and JSON dependency reads receive bounded retries.
- Resolver environment assignments retain Docker's required `-e` argument, including TSX launches that pass the package loader through `NODE_OPTIONS`.
- Permanent dependency permission failures remain environment/module-load blockers, preserve the inaccessible path, and are emitted once before any structured assertion result.

## Evidence and failure semantics

- Zero-test JSON is classified as an environment initialization failure only for recognized runner-internal signatures. Target syntax and top-level import failures remain target-code assertion failures.
- Native or mixed-runner fallback is signature-gated at both transitions. A normal failed attempt is published unchanged rather than replaced by a more permissive execution path.
- Matching-runner fallback uses the selected package's own runner and reports every collected assertion and suite outcome through the existing structured Vitest adapter.

## Red/green evidence

Focused regressions cover package-local runner selection, positive native-failure fallback, target and missing-environment failure preservation, suite lifecycle failures, unsupported filters, JavaScript and JSX instrumentation, loader ordering, transient dependency retry, Docker environment argument construction, and permanent-`EACCES` diagnostic precedence.

Real-project controls were exercised against the Resin8 reproductions:

- The isolated `ProductConfiguration.ts` verification selected the package's Vitest 3.1.3 graph, collected and passed the existing 2/2 authoritative tests, credited three exact target surfaces, and emitted no diagnostics. The overall command remained correctly inconclusive only because seven additional required surfaces lacked behavioral coverage.
- Eight concurrent isolated `useConfigurableProductsFeatureFlag.ts` verifications passed with 1/1 required surface behaviorally checked and zero diagnostics while a separate package verification ran alongside them. No run reported dependency `EACCES`.

## Release validation

The 0.2.12 release candidate passed release-metadata validation, formatting, Clippy with warnings denied, 437 locked Rust tests, 69 benchmark unit tests, 2 release-contract tests, optimized build, CLI and verification smoke checks, package staging, and a fresh held-out matrix dry run. Two focused read-only reviews found no remaining concrete Vitest or isolated-dependency-boundary defect. The release workflow repeats all gates on Linux Node 24 before publishing platform archives.
