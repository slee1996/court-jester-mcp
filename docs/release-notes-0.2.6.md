# Court Jester 0.2.6

Release date: 2026-08-15

Court Jester 0.2.6 fixes the reopened issue #21 reproduction discovered after 0.2.5: a generated TypeScript harness could resolve a workspace package through a package-manager symlink, then fail on that package's extensionless relative imports.

## Workspace-package imports

- The generated Node loader recognizes parent modules in both the temporary project overlay and the canonical source workspace.
- Extensionless `.ts`, `.tsx`, `.js`, `.jsx`, `.mjs`, `.cjs`, and `.json` relative imports resolve from either trusted root.
- Resolution remains containment-checked; paths outside the configured overlay or source root are rejected.
- JSON targets retain the required import attributes.
- Strict Node portability probes bypass the project loader so repository-native resolution does not mask portability warnings.

## Red/green evidence

The focused regression first failed with `ERR_MODULE_NOT_FOUND` for `packages/shared/src/types/tenant` after `@acme/shared` resolved through a workspace symlink. The same exact test passes after the loader fix:

```text
cargo test --locked --test sandbox_test generated_harness_resolves_extensionless_imports_from_workspace_symlink -- --exact --nocapture
cargo test: 1 passed (1 suite, 41 filtered)
```

The strict portability regression also passes independently, confirming that the new resolver does not hide native Node ESM warnings:

```text
cargo test --locked --test verify_test verify_separates_portability_warning_from_execute_success -- --exact --nocapture
cargo test: 1 passed (1 suite, 112 filtered)
```

The release binary was also exercised against the reported monorepo topology:

```text
court-jester verify --file packages/factory-control/src/provenance.ts \
  --language typescript --project-dir /Users/spencerlee/waypoint-mono \
  --summary repair-json

verdict: pass
coverage: 3 required, 3 behaviorally checked, 0 skipped, 0 blocked
```

## Release validation

The candidate passed the complete locked formatting, Clippy, 399-test Rust suite, benchmark unit, release-contract, optimized build, CLI smoke, package-staging, and held-out dry-run gates defined by `just release-check`.
