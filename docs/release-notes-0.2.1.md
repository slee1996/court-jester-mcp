# Court Jester 0.2.1

Release date: 2026-08-01

Court Jester 0.2.1 is a post-0.2.0 correctness and execution-hardening release. It keeps the schema-v3 report contract while making repository context, harness execution, input shape, dependency substitution, and failure provenance consistent end to end.

## Release scope

Compared with v0.2.0, this release focuses on false-failure prevention and auditable execution:

- project-aware source, test, dependency, and package-root resolution across all verification stages;
- Python, TypeScript, TSX, and JSX source-mode selection with structured parse diagnostics;
- typed process termination, resource exhaustion, partial-output, module-load, and network diagnostics;
- hermetic local and isolated harness execution with owned temporary artifacts;
- end-to-end positional, keyword, rest, and keyword-variadic argument preservation;
- deterministic, no-I/O dependency substitutes with explicit provenance and typed unsafe-default skips;
- diagnostic reduction, repair views, replay metadata, and typed benchmark-cause consumers.

## Compatibility

- The active report schema remains version 3.
- Existing 0.2.0 report consumers can continue to deserialize reports; new diagnostic and provenance fields are additive and defaultable.
- Legacy success booleans and legacy failure arrays remain absent from the active contract.
- No crates.io publication is intended; release artifacts continue to ship through GitHub Releases.

## Validation

The release candidate has passed formatting, locked Clippy, release metadata validation, release-contract tests, locked optimized build, package staging, CLI smoke checks, benchmark unit tests, and held-out benchmark-lock validation.

The complete `cargo test --locked --tests` gate is a publication prerequisite and must complete without concurrency-sensitive failures before tagging `v0.2.1`.
