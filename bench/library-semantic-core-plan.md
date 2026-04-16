# Library Semantic Core Plan

This is the next benchmark slice to build if the goal is a **harder but still narrow** proof of Court Jester utility.

The right target is not framework code, network handlers, or concurrency. Court Jester is weakest on code that:

- does not crash
- returns plausible but subtly wrong output
- has a narrow valid domain
- has a stable upstream oracle with many real test cases

That points to **spec-conformance and canonicalization helpers** from widely used libraries.

## Thesis

The strongest next benchmark slice is:

- pure or mostly pure helper functions
- deterministic, offline, and cheap to run
- drawn from heavily used upstream libraries
- backed by real upstream tests, not homegrown assertions

This gives us:

- harder semantic tasks than the current mini-app fallbacks
- much stronger evaluator credibility
- less risk that the benchmark is overfit to Court Jester-shaped fixtures

## What To Target

Build a new task set around **library semantic cores**:

- version semantics
- range/specifier membership
- query-string parse/stringify semantics
- canonicalization and normalization helpers

Do not start with:

- async/concurrency primitives
- request/session lifecycle code
- streaming/network I/O
- timezone databases
- natural-language date parsing
- framework request/response handlers

Those are useful later, but they are a worse first hard slice because they combine semantic difficulty with runtime/world-model difficulty.

## Tier 1: Adopt Now

These libraries are the best immediate sources for a strong evaluator core.

### 1. `pypa/packaging`

Why it fits:

- Python ecosystem-critical
- implements formal interoperability specs like PEP 440
- semantics are exact, not stylistic
- upstream tests are already the oracle we want

Focus areas:

- version ordering
- prerelease handling
- specifier membership
- requirement parsing and normalization

Candidate task families:

- `py-packaging-version-ordering`
  - bug shape: wrong comparison among prerelease, postrelease, and local versions
- `py-packaging-specifier-prerelease-membership`
  - bug shape: incorrectly admitting or rejecting prereleases
- `py-packaging-specifier-compatible-release`
  - bug shape: `~=` or wildcard handling drift
- `py-packaging-requirement-extras-markers`
  - bug shape: parse or normalize requirement strings incorrectly

Why this is a great Court Jester stressor:

- wrong outputs often look reasonable
- failures are semantic, not crash-shaped
- public/hidden splits can come directly from upstream tests

### 2. `npm/node-semver`

Why it fits:

- this is the semver implementation npm uses
- extremely widely depended upon
- semver bugs are exactly the kind of subtle semantic drift Court Jester struggles with

Focus areas:

- comparison
- prerelease ordering
- range membership
- max/min satisfying version selection

Candidate task families:

- `ts-semver-compare-build-vs-prerelease`
- `ts-semver-caret-prerelease-membership`
- `ts-semver-max-satisfying-stable`
- `ts-semver-min-version-range`

Why this is a great Court Jester stressor:

- most wrong answers do not throw
- bugs are usually “almost right”
- upstream tests are deep and trustworthy

### 3. `ljharb/qs`

Why it fits:

- extremely widely used in Node/Express ecosystems
- semantics are compact but tricky
- parse/stringify behavior is easy to evaluate offline

Focus areas:

- nested bracket parsing
- dot notation
- duplicate keys
- array formats
- nullish and empty-value handling

Candidate task families:

- `ts-qs-stringify-array-formats`
- `ts-qs-stringify-empty-nullish-values`
- `ts-qs-parse-nested-brackets-and-dots`
- `ts-qs-parse-duplicate-keys`

Why this is a great Court Jester stressor:

- canonicalization and parsing bugs rarely crash
- edge cases are dense
- upstream parse/stringify tests are already close to benchmark-ready

## Tier 1.5: Keep, But Not As The Hard-Core Focus

### `lodash`

We should keep the existing lodash-derived tasks, but they are not the best next hard semantic slice.

Reason:

- they are useful and credible
- but the current lodash tasks are not as adversarial to Court Jester as semver, qs, and packaging

Use lodash for:

- continuity with existing results
- known-good controls
- broader library-slice reporting

Do not make it the centerpiece of the next narrow hard slice.

## Tier 2: Phase In Later

These are good libraries, but worse first picks for this specific benchmark goal.

### `psf/requests`

Good because:

- widely used
- existing repo-shaped pilot already works

Not ideal as the next core because:

- many interesting behaviors involve request prep, cookies, sessions, and protocol-ish state
- that mixes semantic difficulty with object lifecycle and HTTP assumptions

Best use:

- repo-shaped external pilot tasks
- a small number of helper-level tasks like cookie quoting or URL preparation

### `urllib3/urllib3`

Good because:

- heavily used and foundational
- retry logic and URL helpers are evaluator-friendly

Not ideal as the next core because:

- many attractive behaviors are partially stateful
- retry semantics and redirect/header behavior are more protocolish than pure semantic helpers

Best use:

- phase-2 Python utility slice
- narrow `Retry` or URL helper tasks only

### `dateutil/dateutil`

Good because:

- mature library
- large real test suite
- exactly the kind of subtle semantic space we eventually want

Not ideal as the next core because:

- date parsing and timezone behavior are broad and environment-sensitive
- easy to accidentally build tasks that are hard because of world assumptions, not because of semantic repair

Best use:

- later held-out semantic difficulty slice
- carefully constrained parser or `relativedelta` tasks only

## Recommended New Task Set

Create:

- `library-semantic-core-v1`

Target size:

- `10` to `14` tasks

Suggested composition:

- `4` Python tasks from `packaging`
- `4` TypeScript tasks from `node-semver`
- `4` TypeScript tasks from `qs`

Optional:

- keep `2` existing lodash tasks alongside this set as comparison/context, but do not count them as the main new slice

## Evaluator Construction Rules

This matters more than the task count.

### Public checks

Public checks should be:

- direct adaptations of obvious upstream assertions
- enough to reveal the basic contract
- not enough to fully cover the library behavior

Target:

- `3` to `8` public assertions per task

### Hidden checks

Hidden checks should come from:

1. disjoint held-out upstream assertions first
2. generated cases second, only when upstream coverage is not dense enough

Target:

- `10` to `30` hidden assertions or a small deterministic hidden corpus per task

### Fixture shape

Each task should expose a small wrapper helper, not the whole upstream API.

Good:

- `canonicalize_query(params) -> string`
- `is_allowed(version, specifier) -> bool`
- `max_stable(versions, range) -> string | None`

Bad:

- “reimplement the entire upstream module”
- “parse arbitrary network responses”
- “preserve the whole framework request pipeline”

### Provenance

Every upstream-derived fixture should include `UPSTREAM_NOTES.md` with:

- upstream repo URL
- upstream package/version or tag
- upstream test file(s) used
- which cases became public
- which cases became hidden
- what wrapper contract we froze for the benchmark

## Concrete First Batch

If we only build six tasks first, these should be the six:

1. `py-packaging-version-ordering`
2. `py-packaging-specifier-prerelease-membership`
3. `ts-semver-caret-prerelease-membership`
4. `ts-semver-max-satisfying-stable`
5. `ts-qs-stringify-empty-nullish-values`
6. `ts-qs-parse-nested-brackets-and-dots`

That is a sharp first cut because:

- every task is semantically hard
- every task has a strong upstream oracle
- every task is offline and deterministic
- none of them depends on framework runtime objects

## What This Slice Would Prove

If Court Jester still helps here, the claim gets much stronger:

- it is not just catching crashy business-logic bugs
- it is helping on stable library semantics with credible upstream evaluators
- the benefit is carrying into code that looks correct and often returns plausible wrong answers

## Sources

Official sources to use while building this slice:

- `packaging`
  - PyPI: https://pypi.org/project/packaging/
  - GitHub: https://github.com/pypa/packaging
- `semver`
  - npm: https://www.npmjs.com/package/semver
  - GitHub: https://github.com/npm/node-semver
- `qs`
  - npm: https://www.npmjs.com/package/qs
  - GitHub: https://github.com/ljharb/qs
- `requests`
  - GitHub: https://github.com/psf/requests
- `urllib3`
  - PyPI: https://pypi.org/project/urllib3/
  - GitHub/docs: https://github.com/urllib3/urllib3
- `python-dateutil`
  - PyPI: https://pypi.org/project/python-dateutil/
  - GitHub: https://github.com/dateutil/dateutil
