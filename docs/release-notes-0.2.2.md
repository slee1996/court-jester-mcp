# Court Jester 0.2.2

Release date: 2026-08-14

Court Jester 0.2.2 fixes the three runtime defects found during post-release verification of 0.2.1. It preserves the schema-v3 report contract and the public `TestAdapter::BunJunit` serialization boundary.

## Authoritative TypeScript tests

- Bun authoritative tests now run as `bun test` without incomplete JUnit reporter arguments.
- Network-denied Bun tests keep the preload after the `test` subcommand, preserving Bun's CLI dispatch.
- Default Bun output distinguishes test failures and top-level errors from verifier, network, and process blockers.
- Automatic runner selection recognizes Vitest imports and Vitest-configured packages, including globals-enabled TSX tests.
- Vitest coordinators can create their bounded worker pool while test workers retain Court Jester's network and process-denial guard.

## Isolated doctor

- Docker artifact, workspace, and working-directory paths are canonicalized before containment and container mapping.
- macOS `/var` and `/private/var` aliases no longer make valid generated doctor artifacts appear outside the mirror.
- Canonical containment continues to reject symlink escapes.

## Workspace dependency resolution

- Generated Node and TSX harnesses resolve scoped and unscoped packages from the target package and workspace dependency roots.
- Target package self-references resolve against the candidate overlay rather than stale source on disk.
- Native nearest-package behavior is retained for existing artifacts, sibling workspace packages, and nested dependencies.
- Isolated execution mounts one read-only workspace dependency topology, preserving pnpm relative symlinks without copying `node_modules` into generated overlays.
- Resolver loaders compose with network denial for Node, TSX, and Vitest execution.

## Compatibility

- The active report schema remains version 3.
- `TestAdapter::BunJunit` and its `bun_junit` serialized value remain available.
- No crates.io publication is intended; release artifacts continue to ship through GitHub Releases.

## Validation

The release candidate passed focused red/green regressions for issues #1, #9, and #12, including real Bun execution, isolated Docker doctor checks for Python and TypeScript, local workspace `@prisma/client` resolution, generated/existing pnpm topology checks, Vitest worker-guard behavior, resolver precedence, and path-containment tests.

Publication additionally requires the complete locked formatting, Clippy, Rust test, benchmark unit, release-contract, optimized build, CLI smoke, package-staging, and held-out dry-run gates before tagging `v0.2.2`.
