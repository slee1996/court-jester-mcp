# Verify Evaluation Roadmap

## Current Lane

`verify-mutation-seeds-v1` is the first dedicated verify-recall suite.

- It starts from known-good base fixtures.
- It reapplies seeded bugs by copying source files from the existing buggy repo fixtures during task setup.
- It is intended to run with `noop` plus a verify-running policy such as `advisory`.
- Its headline question is simple: did `verify` reject the seeded bug?

Recommended paired read:

- run `verify-mutation-seeds-v1` for recall
- run `known-good-corpus` for local specificity

## Next Five Lanes

### 1. Bug-Fix Replay Lane

Goal: score `verify` on real pre-fix code from historical bug-fix pairs.

- Source: mined before/after commits from small Python and TypeScript repos
- Main metric: reject pre-fix, accept post-fix
- First step: add a tiny replay pilot with 5-10 manually curated bug-fix pairs

### 2. Differential Oracle Lane

Goal: compare local implementations against trusted reference libraries on generated inputs.

- Source: semver, query-string, packaging, and lodash-style helpers
- Main metric: behavioral agreement rate on generated cases
- First step: wire one semver and one query-string differential harness into `bench/evaluators`

### 3. Metamorphic Property Lane

Goal: test invariants even when no golden implementation exists.

- Source: canonicalizers, normalizers, selectors, comparators, and serializers
- Main metric: property violation rate
- First step: promote a few existing `synthesize.rs` contracts into standalone benchmark fixtures

### 4. Expanded False-Positive Control Lane

Goal: measure how often `verify` blocks code that is already correct.

- Source: more known-good local fixtures plus external gold-patch replays
- Main metric: false-positive rate
- First step: add known-good companions for the new mutation families introduced in v1

### 5. Shadow-Mode Outcome Lane

Goal: validate verifier warnings against real downstream outcomes instead of curated tasks.

- Source: real user-generated patches, tracked without blocking
- Main metric: warning precision against later failures, reverts, or follow-up fixes
- First step: persist verify reports for live runs and join them to later benchmark or product outcomes
