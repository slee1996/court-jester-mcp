# Court Jester 0.2.13

Release date: 2026-08-15

Court Jester 0.2.13 is the publication retry for the immutable 0.2.12 candidate. The 0.2.12 release workflow stopped at Linux Clippy because a macOS-only test helper was imported on every target. This release platform-gates that import without changing the runtime and project-adapter fixes for issues #9, #12, #21, #29, #30, and #31.

## Runtime lifecycle

- Standalone isolated execution creates its workspace through the runtime-profile-aware directory policy. On macOS, default doctor smoke files remain under the Docker-shared home cache for the full container lifecycle instead of using a disappearing `/var/folders` bind source.
- Successful generated TypeScript harnesses exit after emitting `harness_completed` and their final result. Open timers, database clients, or other handles retained by imported project modules no longer turn completed behavioral evidence into a timeout.

## Project-native Vitest correctness

- The portable coordinator resolves `@vitest/runner` from the selected Vitest package before considering a sibling fallback, preventing incompatible workspace-root and leaf-package versions from mixing.
- Project configuration and direct-runner fallback are used only for recognized native or Vitest-internal initialization failures. Target syntax failures, unavailable environments, unsupported filters, and suite lifecycle failures remain authoritative failures.
- JavaScript instrumentation preserves the in-memory payload without unnecessary transpilation; TypeScript, TSX, and JSX-family payloads use the selected project TypeScript package.

## Isolated dependency boundary

- Docker dependency materialization uses one coherent workspace snapshot with package-first resolver roots and package-local transitive dependencies.
- The retrying package loader wraps the TypeScript loader in Node's last-in-first-out loader chain, so transient dependency `EACCES` failures receive bounded retries while permanent permission failures remain environment/module-load blockers.
- Docker environment assignments retain explicit `-e` arguments, and dependency blockers take precedence over structured assertion output.
- Platform-specific native dependencies that are present but incompatible with the container remain environment/module-load blockers after exact target-entry evidence; they cannot become target assertion failures.

## Red/green evidence

Focused regressions cover matching Vitest package selection, signature-gated fallback, suite lifecycle failures, unsupported filters, JavaScript/JSX instrumentation, loader ordering, transient and permanent dependency access, isolated standalone workspace lifetime, and successful harness termination with retained event-loop handles.

Real-project controls passed against the reported reproductions:

- Isolated `ProductConfiguration.ts` selected the package Vitest 3.1.3 graph, passed 2/2 authoritative tests, credited three exact target surfaces, and emitted no diagnostics.
- Eight concurrent isolated Nuxt feature-flag verifications passed with 1/1 required surface checked and no dependency `EACCES` while another package verification ran.
- The exact isolated doctor command from an unrelated workspace passed all five daemon, image, and runtime-smoke checks with default temporary-directory settings.
- Waypoint Fan Atlas direct execution loaded nested workspace JSON and completed generated execution. Its isolated authoritative run resolved `@prisma/client`, preserved exact `syncOne` entry, and classified the host-generated Prisma engine as an environment incompatibility.

## Release validation

The 0.2.13 candidate passed release-metadata validation, formatting, Clippy with warnings denied, 439 locked Rust tests, 69 benchmark unit tests, 2 release-contract tests, optimized build, CLI and verification smoke checks, package staging, and a fresh held-out matrix dry run. The Linux-only import repair also passed the release Clippy command under the pinned Linux Rust 1.86 toolchain before tagging. The release workflow repeats every gate on Linux Node 24 before publishing four platform archives and matching checksums.
