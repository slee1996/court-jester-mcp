# Court Jester 0.2.10

Release date: 2026-08-15

Court Jester 0.2.10 closes the remaining runtime and evidence gaps exposed by reopened issues #9, #21, and #29. Docker-backed doctor checks now retain host-visible support files for the full container lifetime, JSON modules execute through the TypeScript project loader, and Bun project tests can provide exact authoritative reachability in local and isolated profiles.

## Isolated runtime correctness

- Docker harness support files use a stable runtime directory under the Docker-shared macOS home instead of an ephemeral `/var/folders` path that Docker Desktop cannot bind after cleanup.
- Network guards, Node package resolvers, and generated harnesses retain ownership of their temporary directories until the launched process exits.
- Isolated Bun authoritative tests select the pinned `oven/bun:1.3.14` image, invoke Bun inside the container, and materialize the owning package's dependency graph within the project mirror. Host Bun is not required.
- Docker doctor checks continue to enforce read-only project mounts, disabled networking, bounded memory/CPU, and the configured Python and TypeScript images.

## TypeScript project loading

- The Node runtime loader recognizes `.json` modules, parses their contents, and returns a short-circuited ESM default export without requiring import attributes.
- JSON handling applies after TypeScript path-alias and workspace-package resolution, covering direct relative imports, aliased imports, and nested package imports.
- Bun authoritative tests preload target-entry instrumentation in local and Docker execution. The preload emits the same exact `target_entered` protocol events as the Node/Vitest path without rewriting repository source.
- Bun container execution preserves pnpm workspace dependencies and package ownership while keeping generated overlays inside the configured workspace boundary.

## Evidence and failure semantics

- Exact authoritative target-entry events receive authoritative reachability credit independently of generated-harness execution.
- A completed authoritative suite that reaches every required exported surface supersedes an unrelated generated-harness blocker. This prevents a valid project-native test from being downgraded by a synthetic environment mismatch.
- Missing exact target-entry evidence remains inconclusive; test success alone is not treated as target coverage.
- Platform-incompatible generated native dependencies, including a Prisma client built for another operating system, are classified as environment module-load failures rather than target-code contract violations.
- Coverage status distinguishes direct checks, generated calls, factory calls, caller checks, and authoritative-test reachability.

## Red/green evidence

Focused regressions cover macOS Docker bind-source lifetime, isolated Bun without a host Bun executable, pnpm package dependency mapping, direct and aliased JSON imports, Bun preload event emission, authoritative reachability accounting, generated-blocker supersession, and native dependency failure classification.

Real-project controls were exercised against the original Resin8 reproduction workspace:

- The Factory CLI authoritative Bun suite passed locally and in the isolated profile with 2/2 required exported surfaces credited.
- The Fan Atlas suite passed locally with 1/1 required surface credited. In isolated Linux it reached the target, then reported the repository's macOS-generated Prisma client as an environment incompatibility rather than a target failure.
- A nested workspace JSON import passed in the isolated profile with property-checked strength.
- Two consecutive isolated doctor runs from the unrelated Resin8 workspace passed Python and TypeScript runtime smoke checks.

## Release validation

The 0.2.10 release candidate passed release-metadata validation, formatting, Clippy with warnings denied, 425 locked Rust integration tests, 69 benchmark unit tests, 2 release-contract tests, optimized build, CLI/verification smoke, package staging, and a fresh held-out matrix dry run. The release workflow repeats those gates on Linux Node 24 before publishing platform archives.
