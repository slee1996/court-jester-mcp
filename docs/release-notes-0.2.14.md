# Court Jester 0.2.14

Release date: 2026-08-15

Court Jester 0.2.14 expands synthesized verification from a fixed edge-case pass into a layered fuzz campaign. Deterministic domain seeds remain the default foundation; retained-corpus mutation, semantic campaigns, stateful factory sequences, optional native engines, and an opt-in external seed proposer now add depth without weakening sandboxing, finding normalization, minimization, replay, or evidence classification.

## Search depth

- Python and TypeScript analyzers extract predicate-aware seeds from equality, membership, range, and switch/case guards so declared branches enter the initial corpus.
- Generated campaigns retain behaviorally distinct inputs and mutate the corpus instead of repeatedly sampling only fixed edge cases.
- Corpus expansion includes neighboring values, string structure, container shape, nested values, and order mutations while preserving deterministic limits.
- Minimization now reaches an oracle-preserving fixed point and removes independent object fields and collection elements, producing smaller replayable counterexamples.

## Semantic breadth

- Metamorphic campaigns cover idempotency, involution, permutation/order behavior, comparator symmetry, nullish leakage, and round-trip relationships where the analyzed surface supports them.
- Factory-returned callables receive stateful action-sequence campaigns, allowing second-step and later failures to surface with explicit nested invocation identities.
- Campaign output uses the same typed findings and coverage accounting as ordinary synthesized checks; rejected invalid inputs remain distinct from crashes and property violations.

## Optional engines

- `--native-fuzz-engine off|auto|atheris|jazzer` and `--native-fuzz-runs <N>` integrate installed Atheris or Jazzer.js engines behind an explicit opt-in. Missing engines are reported as unavailable rather than silently changing behavior.
- `--llm-plateau-command <PATH>` invokes an external JSON seed proposer only after retained corpus growth stalls. Proposed seeds are bounded, validated, deduplicated, and executed through the normal generated harness.
- Native crashes use the same typed finding, environment-classification, replay, suppression, and report contracts as deterministic findings; externally proposed seeds additionally use the deterministic campaign's fixed-point minimization.

## Evaluation

- `just bench-fuzz-effectiveness` runs seven seeded mutation cases across predicate-aware, stateful, metamorphic, differential, and plateau-escape strategies plus two clean specificity controls.
- The release candidate detected `7 / 7` seeded mutations and kept `2 / 2` clean controls free of findings: mutation recall `1.0`, specificity `1.0`.
- The benchmark verifies the expected finding function/category and stage status, not only process exit codes, and clears each case directory so persisted corpora cannot make repeat runs order-dependent.

## Compatibility and safety

- All deeper search modes remain bounded by the existing timeout, memory, runtime-profile, and network policies.
- Native engines and the external seed proposer are disabled by default. Existing deterministic verification behavior remains the default.
- A plateau-discovered target crash now produces verdict `fail` and exit code `1`; it is no longer misclassified as an infrastructure failure.

## Validation

Release validation runs the version contract, Rust formatting, Clippy with warnings denied, the locked Rust integration suite, benchmark unit tests including the new effectiveness evaluator, release-contract tests, optimized build, CLI smoke checks, package staging, the held-out matrix dry run, and the seeded fuzz-effectiveness lane. The release workflow repeats the release gates on Linux Node 24 before publishing four platform archives and matching checksums.
