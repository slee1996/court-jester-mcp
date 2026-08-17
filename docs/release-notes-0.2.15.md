# Court Jester 0.2.15

Release date: 2026-08-15

Court Jester 0.2.15 repairs seven defects reproduced against the published 0.2.14 standalone artifact. The release converts each reproduction into a red/green regression and strengthens generated fuzz campaigns, project-runner classification, and isolated module loading so verifier behavior preserves the target program's declared contract.

## Type-domain repairs

- `keyof typeof` aliases now inherit finite literal domains from imported Zod-style enum objects. Workspace package exports and wildcard re-exports resolve through the package manifest instead of leaving the alias as `any` (#41).
- Callable-local TypeScript generic constraints are retained in analysis metadata. A parameter such as `T extends ProductRerankCandidate` synthesizes from the constraint rather than an opaque `T`, and `typeof fetch` receives a bounded offline substitute (#42).
- Multiline union continuations remain part of the preceding object field. A callable or array union arm can no longer become a synthetic sibling property such as `| getItems` (#43).
- Callback generators retain their declared return shape, including named array aliases. Valid zero-argument, nullable, authoritative-seed, and planned object calls remain executable when a different surface or union arm is unsupported. Generated TypeScript overlays preserve target bytes. When isolated Linux cannot load a platform-native package through imports that TypeScript would erase, a source-aware loader retry removes only bindings proven to be type-only for that importer, keeps mixed-import runtime bindings intact, and routes an unambiguous named barrel binding directly to its exporting leaf. Other authored runtime imports still evaluate normally (#44).

## Valid-input campaign invariant

Retained-corpus mutation now changes values recursively without deleting object keys or changing tuple/container shape. Generated rows are reported as valid inputs, so mutation must preserve the structural contract that made the seed valid. This prevents a missing required property introduced by Court Jester from being reported as a target-code crash while retaining deterministic value mutation and behavior-signature feedback.

Multiline conjunctive guards now form one complete seed row. A target exception from an exact, unmutated guard-derived valid row is reportable without treating arbitrary generated application `RangeError`s as engine crashes.

TypeScript `typeof` comparisons no longer treat labels such as `"function"` as runtime string inputs. Direct value comparisons in the same predicate remain seed evidence.

## Platform values and project-runner classification

- TypeScript `URL` parameters now receive constructible `URL` instances rather than plain objects. Harness cloning, mutation, shrinking, and repro rendering preserve the class's internal slots, while non-JSON platform objects stay out of persisted corpus JSON (#45).
- A Vitest project-config fallback that executes zero tests because configured globals were lost is a blocking environment/module-loader outcome. It makes the authoritative stage inconclusive instead of reporting a product assertion failure (#46).
- An exact `court-jester process spawn denied` failure in structured Vitest or Jest output under the deny policy is a blocking sandbox diagnostic. Ordinary assertion text and the same message under an allow policy remain target-code failures (#47).

## Regression strategy

The issue-to-release gate covers the published failure modes at three levels:

- analyzer resolution through a workspace package wildcard re-export;
- synthesized generator shape for constrained generics, callback-returning named arrays, and recursive aliases;
- end-to-end verification for finite `keyof` switch domains, nested multiline-union object fields, and planned calls beside unsupported siblings;
- generated-overlay byte identity and import evaluation order, binding-level isolation for type-only platform dependencies while retaining runtime siblings, and selected-export routing through wildcard barrels without evaluating unrelated re-export branches.

The gate also retains adversarial boundaries: genuinely recursive types without a terminating value and alias chains beyond the configured recursion limit remain skipped rather than receiving fabricated empty objects.

## Compatibility and safety

- No report-schema, CLI, sandbox, timeout, memory, or network-policy contract changes.
- Dependency substitutes remain offline and activate only for dependency-shaped parameters.
- Unknown-valued object fields continue to vary, but required sibling fields remain present.

## Validation

The release candidate passes formatting, Clippy with warnings denied, the locked Rust integration suite, benchmark and release-contract tests, optimized build, CLI smoke checks, package staging, and the held-out matrix dry run. Artifact-equivalent Resin8 checks exercise the exact issue #41–#47 target files and project layouts before publication: the private URL helper executes without a fabricated `{}` crash, the lost-Vitest-globals fallback is environment-blocked and inconclusive, and a policy-denied child process is environment-blocked with no gating target diagnostic. The release workflow repeats the release gate on Linux Node 24 before publishing four platform archives and matching checksums.
