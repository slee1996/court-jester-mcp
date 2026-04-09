---
name: authz-security-fuzzer
description: Spec for a Zanzibar authorization model security fuzzer
status: draft
---

# Authz Security Fuzzer — Spec

## Goal

Property-based security testing for Zanzibar-style authorization checkers.
Instead of "does it crash?", verify "does it ever grant unauthorized access?"

## Security Invariants

### 1. Empty Graph → No Access
Construct a `TupleReader` returning empty arrays for all queries.
For any random (object, relation, subject) triple, `checkAccess` must return `false`.
**Violation = phantom permission (critical)**

### 2. Direct Tuple → Access Granted
Insert exactly one tuple: `(object, relation, subject)`.
`checkAccess` for that exact triple must return `true`.
**Violation = broken direct resolution**

### 3. Indirect Chain → Access Granted
Insert tuples forming a valid chain through computed/tupleToUserset rewrites.
`checkAccess` must return `true` for the terminal subject.
**Violation = broken rewrite resolution**

### 4. Tuple Removal → Access Revoked
Start with a graph where access is granted. Remove the granting tuple.
`checkAccess` must now return `false`.
**Violation = stale permission (critical)**

### 5. Cross-Tenant Isolation
Insert tuples for tenant A. Query with tenant B's clientSlug.
`checkAccess` must return `false`.
**Violation = tenant leak (critical)**

### 6. Depth Limit Respected
Create circular tuple chain: A→B→C→A via userset rewrites.
`checkAccess` must terminate and return `false` (not stack overflow).
**Violation = infinite loop / DoS**

### 7. Subject Type Enforcement
Insert a tuple with a subject type not in `allowedDirectSubjectTypes`.
`checkAccess` must return `false`.
**Violation = type bypass**

### 8. Determinism
Same graph + same query → same result across 100 runs.
**Violation = race condition or non-deterministic resolution**

## Graph Generator

```typescript
type FuzzGraph = {
  tuples: AuthzTuple[];
  reader: TupleReader;  // in-memory implementation backed by tuples[]
};

function generateFuzzGraph(schema: AuthzSchema, seed: number): FuzzGraph {
  // 1. Pick random types from schema
  // 2. Generate 5-20 random tuples with valid (type, relation) combos
  // 3. Optionally inject userset chains (subject = type#relation)
  // 4. Build an in-memory TupleReader that filters tuples by query params
}
```

## Oracle

For each invariant, the oracle knows the expected answer independently of the checker:

- **Empty graph**: always `false`
- **Direct tuple**: scan tuples array for exact match
- **Removal**: re-scan after splice
- **Cross-tenant**: filter by clientSlug before scanning
- **Depth limit**: detect cycles in tuple graph via DFS

## Implementation Options

### Option A: New court-jester tool (`authz-fuzz`)
- Dedicated MCP tool that takes schema + checker file path
- Generates graphs, runs invariants, returns structured violations
- Pro: reusable across projects. Con: significant new code.

### Option B: Generated test code via `verify` with `test_code`
- Generate a TypeScript test file that imports the checker and runs invariants
- Pass to existing `verify(file_path, test_code)` pipeline
- Pro: uses existing infrastructure. Con: less structured output.

### Option C: Standalone test file
- Write `tests/authz-security-fuzz.test.ts` in waypoint-mono
- Run with normal test runner (vitest/jest)
- Pro: simplest, lives with the code. Con: not reusable via court-jester.

**Recommendation:** Option C first (prove the invariants catch real issues), then
extract into Option A if the pattern proves valuable across projects.

## Schema-Aware Generation

The fuzzer should read `authzSchema` to generate valid tuples:
- Only use (type, relation) pairs that exist in the schema
- Respect `allowedDirectSubjectTypes` when generating subjects
- Generate userset subjects (`type#relation`) for relations that have rewrite rules
- This ensures the fuzzer tests realistic authorization scenarios, not just random noise

## Priority Order

1. Empty graph (easiest, catches phantom permissions)
2. Depth limit (catches DoS)
3. Cross-tenant isolation (catches worst-case security bug)
4. Direct tuple (catches broken resolution)
5. Tuple removal (catches stale permissions)
6. Subject type enforcement (catches type bypass)
7. Indirect chain (hardest to generate correctly)
8. Determinism (catches race conditions)
