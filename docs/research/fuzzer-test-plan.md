---
name: fuzzer-test-plan
description: Comprehensive test plan for court-jester core fuzzer — 5 areas to cover
status: in-progress
---

# Court Jester — Core Fuzzer Test Plan

## 1. False Negative Testing (CURRENT)

Feed the fuzzer code with known bugs and verify it catches them.

**Python targets:**
- Function that crashes on empty string (`s[0]`)
- Function that crashes on zero (`1/x`)
- Off-by-one in slice (`s[:len(s)+1]` is fine, `s[len(s)]` crashes)
- Type coercion bug (assumes int, gets float with decimal)
- None propagation (missing null check on optional field)
- Unicode handling (crashes on multi-byte chars, surrogate pairs)
- Large input (quadratic behavior that hits timeout)
- Regex catastrophic backtracking

**TypeScript targets:**
- `Cannot read properties of undefined` (missing optional chaining)
- `toString()` on null/undefined
- Array index out of bounds
- parseInt without radix returning NaN propagation
- Object spread overwriting critical fields
- Promise rejection not caught (async function)

**Method:** Write each buggy function, run verify, assert fuzz stage catches it
(exit_code != 0 or fuzz_failures non-empty).

## 2. Harness Correctness

Verify generated Python/TypeScript harness code is syntactically valid and
exercises what we think it does.

- Generate harness for 20+ real-world function signatures
- Verify generated code parses without syntax errors (run through tree-sitter)
- Verify each function name appears in a FUZZ label
- Verify edge cases are included for typed params (int→EDGE_INTS, str→EDGE_STRS)
- Verify keyword-only args use `name=` syntax
- Verify methods and nested functions are NOT called directly
- Verify factory exercise block appears for functions with nested children

## 3. Property Inference Accuracy

Test that property checks trigger correctly and don't produce false positives.

- Idempotency: verify `clean_text` triggers, `double` does not
- Boundedness: verify `trim` triggers, `pad` does not
- Non-negativity: verify `count_items` triggers, `difference` does not
- Symmetry: verify `distance(a,b)` triggers, `concat(a,b)` does not
- Involution: verify `encode`/`decode` pair triggers roundtrip check
- False positive rate: run against 50+ real functions, count spurious property violations

## 4. Import Resolution Coverage

Test across more import patterns to ensure type resolution works.

- Re-exports (`export { Foo } from "./other"`)
- Barrel files (`import { Foo } from "./index"`)
- Deep chains (`a.ts` imports from `b.ts` which imports type from `c.ts`)
- Aliased imports (`import { Foo as Bar } from "./types"`)
- Default exports (`import Foo from "./types"`)
- Python relative imports (`from .module import Foo`, `from ..parent import Bar`)
- Python dataclass fields (already working)
- TypeScript type aliases with generics (`type Result<T> = { data: T; error?: string }`)
- Union type aliases (`type ID = string | number`)

## 5. Analyzer Edge Cases

Test that the tree-sitter analyzer handles unusual but valid code patterns.

- Async functions and generators (`async function`, `function*`)
- Decorators (`@dataclass`, `@app.route`)
- Overloaded TypeScript signatures
- Generic functions (`function identity<T>(x: T): T`)
- Destructured parameters (`function foo({ a, b }: Options)`)
- Rest parameters in both languages
- Default exports (`export default function`)
- IIFE patterns
- Class static methods
- Property access chains in type annotations (`Foo.Bar.Baz`)
- Conditional types (`T extends U ? A : B`)
