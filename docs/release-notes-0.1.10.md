# Court Jester 0.1.10

Date: 2026-04-22

## Summary

This release turns the recent CI-adoption work into the shipped contract.

Since `0.1.9`, Court Jester reports a versioned JSON schema, gives teams finer execute gating and suppression controls, separates diagnostics from failures more cleanly, and reaches more real inputs through auto-seeding from simple call sites and nearby tests.

## Highlights

- CI-facing report shape is now explicit and versioned.
  - verify reports include `schema_version: 2`
  - `--report-level full|minimal` lets CI keep only the stable, high-signal fields
- Execute-stage gating is more usable in real workflows.
  - `no_inputs_reached` is diagnostic by default instead of failing the stage
  - `--execute-gate all|crash|none` lets teams fail only on crash findings if needed
  - `--suppressions-file <PATH>` keeps known findings visible without forcing teams to disable whole stages
- TypeScript and complexity signals are more operational.
  - portability reports now expose structured `reason`, `failing_imports`, and `fix_hint`
  - `--complexity-metric cyclomatic|cognitive` lets teams gate on either metric explicitly
  - execute findings can carry `classification: "type_signature_wider_than_usage"` when observed literal call sites suggest the declared type is wider than real usage
- Coverage accounting is more honest and more productive.
  - zero-argument functions with non-primitive return surfaces can report `skipped_no_fuzzable_surface`
  - verify can auto-seed fuzzing from simple literal call sites in the source file and nearby conventional test files
  - `--no-auto-seed` disables that path when users want a stricter pure-generation run
- Installer UX is clearer for first-time TypeScript users.
  - the public install script now prints a Biome follow-up when no sibling or `PATH` Biome is available

## Validation

Validated for this release:

- full Rust test suite with the pinned repo toolchain:
  - `/bin/zsh -lc 'RUSTC=$(rustup which rustc) RUSTDOC=$(rustup which rustdoc) rustup run stable cargo test -- --nocapture'`
- installer syntax check:
  - `sh -n install.sh`
- targeted regression coverage was added for:
  - auto-seeding from nearby tests
  - zero-argument no-fuzzable-surface classification
  - type-signature-wider-than-usage execute classification
  - cognitive complexity gating
  - suppressions and execute severity gating

## Known Limits

- TypeScript authoritative `--test-file` runs under Node. Bun-native test files that import `bun:test` are not yet supported directly as the authoritative stage.
- TypeScript lint still depends on a project-local, sibling, or `PATH` Biome binary; the installer only prints the follow-up, it does not download npm packages for you.
- `court-jester ci` is still the main planned workflow gap after this release.
