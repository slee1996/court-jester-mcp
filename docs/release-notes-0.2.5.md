# Court Jester 0.2.5

Release date: 2026-08-15

Court Jester 0.2.5 publishes the active verifier correctness backlog: issues #1, #9, and #15 through #26. It supersedes the unpublished 0.2.3 and 0.2.4 candidates after their quality workflows exposed concurrent child-process test races. The release preserves the schema-v3 report contract while tightening runtime selection, project-context fidelity, synthesis, lossless reporting, and inconclusive/failure classification.

## Repository-native TypeScript tests

- `test_file_path` execution uses the repository's Vitest, Jest, or Bun test coordinator rather than plain Bun script mode.
- Vitest launchers are resolved through their owning package manifest, including scoped aliases and global/package-manager symlinks, without executing arbitrary wrapper scripts.
- Vitest 0.x keeps its legacy `--threads false` contract; modern Vitest uses a bounded fork pool. Network and process denial remain active in test workers.
- Reporter JSON can be extracted from bounded mixed stdout containing test logs or warnings. Malformed and non-result output still fails closed as a harness protocol error.

## Runtime and project context

- Isolated doctor runs preserve a Docker Desktop-accessible lexical bind path while canonical source, project, working-directory, and artifact containment still rejects symlink escapes.
- TypeScript harnesses honor extended JSONC `tsconfig` files, `baseUrl`, exact aliases, and most-specific wildcard alias precedence while preventing mapped targets from escaping the project mirror.
- Nuxt auto-import setup failures are classified as environment/inconclusive, including failures before the harness bootstrap event. Projects that explicitly disable auto-imports retain normal target-error classification.
- Python fuzz, differential, and persisted replay harnesses load target code under a non-main module identity. Guarded `argparse`, CLI, server, and worker entry points no longer run during verification.

## Synthesis and oracle correctness

- TypeScript array parameters receive arrays whose elements match the declared type, including empty and multi-element edge cases.
- Primitive default initializers inform parameter synthesis when no explicit annotation is present. BigInt defaults fail closed rather than being misclassified as numbers.
- Imported non-null object aliases resolve to compatible object generators. Unresolved recursive aliases and aliases that exhaust the recursion limit are skipped instead of receiving invalid `{}` or `null` values.
- Implicit consistency checks are disabled for detected random, time-based, and mutable-state behavior. Pure transforms remain checked even when local bindings shadow module-state names.
- Python semantic equality treats freshly created callable objects and callable containers by stable semantic identity instead of raw process identity.

## Lossless findings and diagnostics

- JavaScript repro values preserve `undefined`, `NaN`, `Infinity`, and `-Infinity` with recursive tagged encoding. Ordinary objects that resemble reserved tags remain distinct during reporting and shrink deduplication.
- The safe, unshadowed `Object.prototype.hasOwnProperty.call(target, key)` pattern no longer triggers `noPrototypeBuiltins`. Unsafe target-owned calls, nested unsafe arguments, and locally shadowed `Object` bindings remain reportable.
- Signal-terminated Ruff runs are environment failures, not lint findings. Diagnostics include the executable, argument vector, target path, working directory, signal, and safe rerun guidance without dumping environment variables or source contents.
- Verification emits an explicit skipped execute stage for parse failures or when no runnable targets exist; the top-level verdict can no longer silently pass without execution evidence.
- MCP-facing errors retain a stable message, stage, and diagnostics instead of producing message-less protocol failures.

## Release validation

- Human-summary formatting is covered by an in-memory report fixture rather than concurrent Node fuzz execution. The formatter assertions are unchanged, but no runtime scheduling can turn this unit test red.
- Rust integration targets run with one test thread in the canonical local and CI release gates. Runtime-heavy tests create real child-process sandboxes, so serial execution prevents scheduler/resource contention from being misread as a product verdict.
- The active report schema remains version 3.
- Existing runner selection and serialized adapter values remain supported.
- Release artifacts continue to ship through GitHub Releases only; no crates.io publication is intended.

## Validation

The integrated candidate passed 395 Rust tests across 15 test suites. Focused red/green regressions cover every issue and the adversarial follow-up cases for shadowed state, BigInt and empty-array defaults, unresolved imported aliases, tag collisions, Python replay, lint span filtering, mixed Vitest output, `baseUrl` and alias precedence, JSONC strings, Nuxt pre-bootstrap failures, disabled auto-imports, recursive aliases, and deterministic human-summary rendering.

Real-runtime smoke evidence includes:

- isolated Python and TypeScript doctor checks passing from an unrelated monorepo under Docker Desktop;
- the Resin8 Vitest 0.19.1 suite running through the project coordinator with 11/11 tests passing and JSON recognized (the separate authoritative-surface instrumentation gate remains inconclusive for that external suite);
- a guarded Python `argparse` module completing 36 generated cases without executing its `__main__` block;
- the reproduced macOS Ruff SIGKILL returning an actionable environment diagnostic rather than a lint violation;
- representative Resin8 Nuxt and `tsconfig`-alias modules returning explicit skipped/inconclusive outcomes without false target-code findings.

The release candidate passed the complete locked formatting, Clippy, serialized 395-test Rust suite, 69-test benchmark suite, two release-contract tests, optimized build, CLI smoke, package-staging, and held-out dry-run gates before tagging `v0.2.5`.
