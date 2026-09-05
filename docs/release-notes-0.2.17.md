# Court Jester 0.2.17

This release strengthens the changed-code counterexample → repair → replay workflow. It does not claim general correctness, complete framework coverage, or measured agent productivity gains.

## User-visible changes

- Input admission and completed-check evidence are handled separately from generated inputs and mere target entry. Exception names or engine-like messages alone do not establish a target bug. Unknown-input failures remain observations and can make verification inconclusive.
- Direct, property, semantic, paired-function, and factory replay retain the relevant arguments, observations, action history, receiver, and failure phase. A different exception or an earlier failure does not impersonate the saved finding. TypeScript factory setup and actions are awaited, including ordinary functions returning promises.
- Differential replay accepts `--candidate-project-dir` to compare the current candidate against the saved baseline. Historical replay remains available. Live regression export requires fresh closed-domain admission and conclusive comparison evidence.
- Opt-in `--export-regression` creates a regression bundle for eligible findings. Positive check evidence is required after repair; failure to reproduce alone is not proof that the check passed. Inferred expectations require explicit acceptance, and unknown-input factory observations are not exportable expectations.
- Native adapters snapshot supported runtime inputs before target mutation. Versioned replay contracts separate arguments from the failure check. Bounded minimization retains only fresh-process reproductions, preserves runtime types and admission, and keeps original and best-case evidence. Its shared cap is five seconds within the remaining native-stage allowance and 32 managed operations; it is not a global-minimum guarantee.
- Repository defaults, source-specific test routing, and validated suppressions support predictable verification. CI distinguishes working-tree inputs from exact committed Git blobs, with retained replay workspaces.
- Project-aware doctor probes inspect configured entrypoints without running them by default. Opt-in local and isolated probes distinguish missing dependencies, failed runtime checks, and positive target-load evidence. Fresh-project onboarding gates cover Python and TypeScript.
- Managed processes retain process-group and output-collector ownership across cancellation. Container workers retain workspace leases until bounded exact-container cleanup. CLI SIGINT/SIGTERM allow owned cleanup; SIGKILL, runtime destruction, and unavailable daemons remain limitations.

## Compatibility and limits

The same target code may now produce fewer bug claims or an inconclusive verdict because input admission or completed-check evidence is missing. This is an intentional correction, not a weakened gate. Native legacy records without a supported binding contract cannot be minimized; unsupported runtime values abstain. Saved report schema validation still applies.

Automatic factory sequences have no general lifecycle-admission contract. General Python coroutine support and broader callable/lifecycle coverage remain unfinished. Advisory test-quality outcomes remain separate from verification verdicts and are not an adequacy score. Retained dependency workspaces are not hermetic dependency snapshots.

## Verification and release status

The implementation candidate at `5896d76` passed hosted quality CI, including Rust integration, isolated-runtime checks, release-binary smoke, repair, onboarding, classification, effectiveness, and locked benchmark dry-run gates. Local broad verification recorded 720 passes; two filesystem-restricted checks passed separately with the required access. Async factory bug/replay/repair behavior was also exercised in isolated Node.

These results precede the version bump. The versioned candidate must pass `just release-check` (or its exact commands when `just` is unavailable) and hosted CI before tagging. The release workflow builds macOS/Linux ARM64 and AMD64 archives and checksums. Platform artifacts have not yet been built for this version. Actual paired agent-repair, latency/cost evidence, and design-partner outcomes remain pending; deterministic fixtures do not substitute for those results.

Distribution remains GitHub Releases only. This preparation does not publish to crates.io, create a tag, or publish a GitHub release.
