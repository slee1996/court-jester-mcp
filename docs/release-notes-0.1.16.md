# Court Jester 0.1.16

Date: 2026-04-25

## Summary

This release improves Court Jester's synthesized execute stage by making input generation more domain-aware before fuzzing. The verifier now walks more of the code and type tree to find closed input domains from literal annotations, TypeScript enums, and `as const` tuple aliases.

## Highlights

- Python `typing.Literal[...]` annotations now generate declared literal values, including nested collection elements such as `list[Literal["create", "delete"]]`.
- TypeScript literal unions and object fields with literal domains now generate declared branch values.
- TypeScript enum declarations are exposed to synthesis as literal-union aliases.
- TypeScript `typeof CONST_TUPLE[number]` aliases are rewritten from `as const` array declarations.
- Imported enum and const-tuple alias context is resolved through the existing local type-resolution path.
- Closed literal-domain object inputs avoid broad `{}` edge cases that fall outside the declared shape.

## Validation

Validated for this release:

- `/bin/zsh -lc 'RUSTC=$(rustup which rustc) rustup run 1.86.0-aarch64-apple-darwin cargo test --locked --test analyze_test --test synthesize_test --test verify_test'`
- `/bin/zsh -lc 'RUSTC=$(rustup which rustc) rustup run 1.86.0-aarch64-apple-darwin cargo build --release --locked'`
- `python3 -m py_compile scripts/smoke_cli.py bench/autoresearch_signature_contracts.py bench/autoresearch_terminal_bench.py`
- `python3 scripts/smoke_cli.py --release --verify-sample`
- `python3 -m bench.autoresearch_signature_contracts --task-set external-heldout-synth-guardrail-v4` buggy and fixed-gold lanes
- Replayed saturated external guardrails `v1`, `v2`, and `v3` buggy and fixed-gold lanes
- `python3 -m bench.autoresearch_terminal_bench --limit 40`

Latest guardrail results:

- `v4`: 4 true positives on buggy fixtures, 4 true negatives on gold-patched fixtures, 0 false positives.
- `v3`: 4 true positives on buggy fixtures, 4 true negatives on gold-patched fixtures, 0 false positives.
- `v2`: 8 true positives on buggy fixtures, 8 true negatives on gold-patched fixtures, 0 false positives.
- `v1`: 9 true positives on buggy fixtures, 9 true negatives on gold-patched fixtures, 0 false positives.
- Terminal-Bench QuixBugs slice stayed stable: generic fuzz 13 clean true positives; normalized tests-only 28 clean true positives.

## Known Limits

- This release improves closed-domain input synthesis. It does not add a general semantic oracle for arbitrary enum-like business logic.
- Terminal-Bench aggregate results are intentionally unchanged; this release is about typed repo-code domain inference, not QuixBugs-style untyped algorithm tasks.
