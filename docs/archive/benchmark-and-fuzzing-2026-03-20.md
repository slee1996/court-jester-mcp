# Court Jester Benchmark And Fuzzing Memo

Date: 2026-03-20

## Summary

Court Jester is increasingly validating as an in-loop breaker for code agents.

The current product shape that is paying rent is:

- `baseline`: model solves the task with no Court Jester intervention
- `required-final`: useful as a control, but not the product target
- `repair-loop`: the main path, where `verify` breaks a bad first attempt and one repair turn gets a chance to recover

The benchmark work so far has been less about making a final checker prettier and more about making the verifier harsher, more realistic, and less noisy.

The headline result pattern is now consistent:

- one repair turn is the strongest policy
- hard gating is weaker because it catches bad first attempts without allowing recovery
- two repair turns are still too brittle to default on

## What We Have Done So Far

### 1. Built a real local benchmark harness

We built a benchmark harness around Court Jester rather than testing the verifier in isolation.

Core pieces:

- `bench/run_matrix.py`
- `bench/runner.py`
- `bench/providers.py`
- `bench/summarize_runs.py`

That harness now supports:

- local fixture repos
- paired `task x model x policy x repeat` runs
- Codex CLI and Claude CLI providers
- public checks
- hidden evaluators
- Court Jester MCP verification
- result provenance and artifact capture

### 2. Established the product direction with repeatable policies

We benchmark three main policies:

- `baseline`
- `required-final`
- `repair-loop`

We also keep `repair-loop-2` around as an experimental pressure test, but it is no longer the product center.

This mattered because the benchmark repeatedly showed:

- `required-final` can catch real bugs, but it throws away recovery
- `repair-loop` converts at least some of those catches into successful final outcomes
- extra repair turns often introduce more instability than value

### 3. Hardened the verifier against obvious false positives

Early benchmark runs exposed several verifier and harness problems that made Court Jester look worse than it really was.

Important fixes included:

- lint warnings became informational instead of hard failures
- TypeScript execute timeouts were stabilized with a Node-first path and longer timeout
- Python and TypeScript file-based verify tests were fixed to import target modules correctly
- helper files stopped inheriting unrelated public verify files
- TypeScript semver helpers stopped being fuzzed with impossible states
- query-string serialization now catches nullish sentinel leakage like `"None"` or `"undefined"`

These fixes were not speculative cleanup. They came directly from failing benchmark artifacts.

### 4. Expanded the task suite toward harder semantic failures

The benchmark started with small local semantic bugs and moved toward more adversarial and SWE-bench-shaped tasks.

Task families now include:

- fallback-chain logic
- nested nullable object handling
- sparse array and empty-string behavior
- cross-file helper/caller contracts
- semver-style comparator/range/max logic
- feature-flag precedence with explicit false overrides
- canonical serialization and query-string normalization

The point of these tasks is not to imitate public benchmark branding. The point is to hit the kinds of subtle semantic failures that strong code agents still make.

### 5. Tightened measurement validity

We improved the benchmark so small policy deltas are more trustworthy.

Changes:

- paired hidden seeds across policies for the same `task x repeat`
- explicit provenance in results:
  - `verify_failed`
  - `public_failed`
  - `hidden_failed`
  - `failure_provenance`
  - `repair_trigger_source`
- hidden evaluation can be sampled on obvious public failures instead of always burning compute
- `required-final` is now treated as a control in summaries, not the headline product metric

### 6. Validated real repair-loop wins

The strongest current evidence is no longer anecdotal.

Examples:

- `py-display-handle-fallback`: Codex `baseline 1/3`, `required-final 1/3`, `repair-loop 3/3`
- `py-query-string-canonicalization`: after query-string hardening, Codex `baseline 2/3`, `required-final 1/3`, `repair-loop 3/3`
- `ts-semver-max-stable-cross-file`: after fixing the false-positive path, the task became clean again across both models and policies

That is the exact behavior we want:

- baseline can miss
- hard gate can catch the miss and fail
- one repair turn can catch the miss and still converge

## Our Fuzzing Approach

Court Jester fuzzing is not blind random input generation. It is type-aware, name-aware, and benchmark-driven.

The goal is to generate failure-inducing inputs that are plausible enough to matter and sharp enough to reveal hidden semantic mistakes.

### 1. Type-aware input synthesis

We synthesize inputs from the function signature and lightweight code analysis.

That includes:

- Python scalar, dict, list, and nested container shapes
- TypeScript primitives, unions, arrays, inline object members, and nullable aliases
- special handling for common shapes like `Record<string, unknown>`, `dict[str, object]`, `Array<string | null | undefined>`, and nested object literals

This is what keeps the verifier from acting like a useless generic fuzzer.

### 2. Edge-case corpora for common failure classes

We maintain explicit edge cases for patterns that code agents regularly mishandle.

Examples:

- blank-but-present strings
- whitespace-only strings
- nested `None` / `null`
- sparse arrays
- nullable list items
- explicit false overrides
- accent normalization and canonical serialization
- semver-shaped objects with constrained numeric fields

These are the inputs that drive the useful failures.

### 3. Name-cued semantic properties

We do not only generate inputs. We also attach properties based on function names and return shapes.

Examples:

- label / display / city helpers should not collapse to empty strings in the wrong cases
- canonical / serialize / query helpers should not leak `None`, `null`, or `undefined` into output strings
- semver helpers should operate on realistic parsed-version domains, not arbitrary junk

This is how Court Jester acts more like a hostile semantic checker than a syntax checker.

### 4. Language-specific fuzz templates

Python and TypeScript use different fuzz harnesses because the failure modes differ.

Python side:

- executes synthesized calls directly
- checks crashes and semantic property violations
- uses helper predicates like `_contains_nullish` and `_string_leaks_nullish`

TypeScript side:

- runs generated cases in the sandbox
- checks explicit properties inside `_fuzzOne`
- treats property violations as failures, not style signals

We keep the two languages aligned conceptually but not mechanically identical.

### 5. File-based public verify tests as bounded pressure

For some tasks, Court Jester also runs a small file-based verify test.

That is not the same as hidden evaluation:

- public verify tests are cheap and visible
- hidden evaluators are the final correctness measurement

The verify test stage is useful when it provides fast, concrete breakage without leaking the full hidden contract.

### 6. Hidden evaluator after the verifier

The hidden evaluator is where we measure end-to-end correctness.

Today it is still local and only partially obscured. That means it is not a true secrecy boundary.

Still, it is already useful because:

- hidden checks are separate from Court Jester logic
- seeded hidden generation lets us vary cases per repeat
- policy comparisons now use paired seeds

This gives us practical product signal without pretending we already have a fully external judge.

### 7. Benchmark-driven fuzzing, not theory-driven fuzzing

The most important part of the fuzzing approach is feedback.

We use failing benchmark artifacts to decide what to add next.

Recent examples:

- semver cross-file false positive -> narrowed TS array/object edge handling
- semver caret helper mismatch -> constrained semver numeric generators and removed bad idempotence assumptions
- query-string canonicalization miss -> added nullish-leak properties and query-shaped dict/list edge cases

In other words: the fuzzing strategy evolves from real misses, not from abstract completeness goals.

## What We Believe Now

The best current description of Court Jester is:

> a hostile verifier that should break bad AI-generated code early enough for one repair turn to recover

Not:

> a hard final gate that should block completion with no recovery

That distinction matters because it changes both product design and benchmark interpretation.

## Known Gaps

The system is better than it was, but it is not done.

Current gaps:

- hidden evaluation is still local obscurity, not a true external judge
- public-triggered live repairs are still rare because the current public tasks are often too easy for strong models
- `required-final` remains a useful control but not a great product experience
- `repair-loop-2` still looks worse than a single repair turn on harder suites
- benchmark throughput could improve, but measurement validity still matters more than speed right now

## Near-Term Plan

Next work should stay centered on the same thesis:

1. keep expanding harder task families
2. keep fixing verifier mismatches exposed by those tasks
3. keep measuring `repair-loop` as the headline
4. keep `required-final` as a control
5. move toward a real external hidden evaluator once the local benchmark signal is strong enough

## Current Benchmark Stance

If we had to summarize the system in one line today:

> Court Jester is starting to validate as a one-repair-turn breaker for code agents, and the benchmark is now good enough to keep finding where it still falls short.
