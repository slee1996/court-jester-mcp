# Express Testfile Gauntlet v1 Plan

This is the first executable benchmark phase on the path to cloning all of Express.

It is not the final goal. The final goal is still:

- one clone repo
- one coherent Express-compatible API surface
- broad upstream test parity

But the first phase has to be structured tightly enough that we can learn where Court Jester actually helps before we spend weeks building the full fixture.

This document defines that phase.

## What "Clone All Of Express" Means

The benchmark target is not "build something vaguely like Express."

The benchmark target is:

- source of truth: upstream `expressjs/express`
- behavioral bar: pass the selected upstream Express test surface without rewriting the tests
- end-state milestone: cover the full current upstream file-level test surface, which is roughly `70` test files across app/router, request, response, and wrapper APIs

So the roadmap is:

1. `express-testfile-gauntlet-v1`
2. `express-full-clone-alpha`
3. `express-full-clone-beta`
4. `express-full-clone-v1`

The gauntlet is phase 1, not the headline destination.

## Why This Phase Exists

The current benchmark harness is good at small semantic tasks. That is not enough.

For Express, we need to know:

- can Court Jester help on real framework behavior
- can it compete with tests-only repair on framework-shaped tasks
- which Express surfaces actually trigger verifier value
- where public tests are enough and where they are not

The gauntlet lets us answer those questions with upstream provenance and clean failure attribution.

## Scope

`express-testfile-gauntlet-v1` should be:

- `12` tasks
- all sourced from upstream Express test files
- one benchmark task per upstream file or tightly coupled test family
- all JS/TS
- each task uses a small Express-clone fixture rather than a toy unrelated service

The first gauntlet should intentionally skew toward the hardest product-relevant surfaces:

- router dispatch
- mount behavior
- param processing
- error propagation
- request query/base-url semantics
- response content negotiation and redirect semantics

Do not spend the first wave on the easiest helper cases.

## Task Families

Each task should preserve direct provenance to one upstream file.

### Bucket A: Public-Fire Router And Middleware Tasks

These should make tests-only repair actually fire.

1. `express-router-dispatch`
   - upstream: `test/Router.js`
   - core behaviors:
     - nested router mounting
     - dynamic params through mounted routers
     - missing URL / missing method handling
     - large stack traversal without stack overflow
   - why it matters:
     - this is real framework control flow, not a helper method

2. `express-app-use-mounting`
   - upstream: `test/app.use.js`
   - core behaviors:
     - app mounting at mount points
     - dynamic mount paths
     - middleware ordering around mounted apps
     - `mount` event and `parent` wiring
   - why it matters:
     - mount ordering bugs are easy to write and hard to spot visually

3. `express-app-param-routing`
   - upstream: `test/app.param.js`
   - core behaviors:
     - per-param mapping
     - once-per-request semantics
     - distinct values across routes
     - `next('route')`
     - error behavior from param hooks
   - why it matters:
     - sequencing and route-skipping semantics are exactly where plausible clones go wrong

4. `express-app-routes-error`
   - upstream: `test/app.routes.error.js`
   - core behaviors:
     - error handlers only fire on propagated error
     - non-error handlers are skipped on error
     - error chain termination behavior
   - why it matters:
     - error middleware is core framework semantics, not optional sugar

### Bucket B: Mixed Public/Hidden Routing Context Tasks

These should allow public repair to engage on canonical cases while leaving deeper hidden edges.

5. `express-req-baseurl-traversal`
   - upstream: `test/req.baseUrl.js`
   - core behaviors:
     - empty top-level `baseUrl`
     - lower-path accumulation through nested routers
     - baseUrl progression through multiple middleware layers
   - why it matters:
     - mounted routing context is subtle and central to large Express apps

6. `express-req-query-parser-modes`
   - upstream: `test/req.query.js`
   - core behaviors:
     - default simple parsing
     - extended parser behavior
     - disabled parser behavior
     - custom parser function behavior
     - unknown parser setting throws
   - why it matters:
     - realistic config-driven semantics with easy hidden-edge mistakes

### Bucket C: Hidden-Edge Response Semantics

These are likely to pass naive public tests while still containing semantic mistakes.

7. `express-res-format-negotiation`
   - upstream: `test/res.format.js`
   - core behaviors:
     - q-value content negotiation
     - wildcard matches
     - `Vary: Accept`
     - charset defaults
     - `.default` behavior
     - `406` with supported types list
   - why it matters:
     - this is exactly the kind of plausible-but-wrong semantic logic Court Jester needs to prove it can help with

8. `express-res-redirect-rendering`
   - upstream: `test/res.redirect.js`
   - core behaviors:
     - default `302`
     - status overload
     - URL encoding and already-encoded sequences
     - HEAD body omission
     - html/plain/empty-body response negotiation
     - redirect-body escaping
   - why it matters:
     - combines content negotiation, header setting, and escaping in one surface

9. `express-res-send-body-shape`
   - upstream: `test/res.send.js`
   - core behaviors:
     - string vs buffer vs object handling
     - content-type defaults
     - status/body interactions
     - HEAD behavior
   - why it matters:
     - easy place for clones to look correct but diverge on wire behavior

10. `express-res-json-and-jsonp`
    - upstream: `test/res.json.js`, `test/res.jsonp.js`
    - core behaviors:
      - JSON serialization
      - escaping behavior
      - callback-wrapping behavior
      - content-type details
    - why it matters:
      - this is widely used surface area, not edge-case plumbing

### Bucket D: Wrapper API And Boundary Tasks

These push toward product-shaped completeness.

11. `express-wrapper-json-urlencoded`
    - upstream: `test/express.json.js`, `test/express.urlencoded.js`
    - core behaviors:
      - wrapper API wiring
      - content-type gating
      - parse result placement
      - failure behavior on malformed bodies
    - why it matters:
      - high user-facing product value

12. `express-wrapper-static-and-route`
    - upstream: `test/express.static.js`, `test/Route.js`, optionally `test/app.router.js`
    - core behaviors:
      - route object semantics
      - route stacking
      - static wrapper integration on basic cases
    - why it matters:
      - forces the clone to look more like a framework product and less like a hand-written router

## Public / Hidden Strategy

The gauntlet only works if public repair actually fires and hidden edges still matter.

Per task, use:

- public:
  - `1-3` canonical upstream cases
  - enough to make tests-only repair fire on at least some tasks
- hidden:
  - `3-8` deeper upstream cases from the same file
  - neighboring regressions from the same subsystem

Do not:

- dump the full public test file into the prompt
- hide the entire test file wholesale
- let hidden failure trigger repair in the main comparator

The main policy comparator remains:

- `baseline`
- `public-repair-1`
- `repair-loop-verify-only`
- `public-repair-2`
- `repair-loop-verify-only-2`

## Full Clone Milestones

The gauntlet is only valid if it rolls up into a real full-clone program.

### `express-full-clone-alpha`

Target surfaces:

- `test/Router.js`
- `test/Route.js`
- `test/app.use.js`
- `test/app.param.js`
- `test/app.routes.error.js`
- `test/req.baseUrl.js`
- `test/req.query.js`
- `test/res.format.js`
- `test/res.redirect.js`
- `test/res.send.js`
- `test/res.json.js`

This is the first product-shaped milestone: core router, mounting, request context, and response negotiation.

### `express-full-clone-beta`

Adds:

- remaining request helpers
- remaining response helpers
- `express.json`
- `express.urlencoded`
- `express.static`
- broader regressions

### `express-full-clone-v1`

Pass the full committed upstream Express test surface in this repo, targeting the current roughly `70` file-level upstream test modules cataloged in [express-full-clone-plan.md](/Users/spencerlee/court-jester-mcp/bench/express-full-clone-plan.md).

## Benchmark Operator Gates

Do not promote the gauntlet to the headline large-scale benchmark unless the pilot shows all of these:

1. `baseline` success is neither trivial nor hopeless.
   - target band: roughly `30%` to `80%`

2. `public-repair-*` actually fires.
   - at least `20%` of public-fire task/model cells should take a public-triggered repair attempt

3. `repair-loop-*` actually fires on hidden-edge tasks.
   - verify-triggered repairs should appear on mixed or hidden-edge buckets

4. provider noise stays low enough to interpret policy differences.

If public repair does not fire, the suite is not a valid tests-vs-verify comparison yet.

## Metrics That Matter

Primary:

- final success rate
- additional successes vs baseline
- product minutes per success
- marginal minutes per extra saved task

Comparator-specific:

- public-triggered repair count and recovery rate
- verify-triggered repair count and recovery rate
- success on hidden-edge tasks
- success on tasks that pass public but fail hidden on baseline

Trace-derived mechanism metrics:

- average trace event count on repair attempts
- average search commands on repair attempts
- average file-read commands on repair attempts
- average files touched per repair attempt

The strongest claim we should aim for is:

- on real Express-derived framework tasks, Court Jester beats or matches tests-only repair and reaches saved tasks with less search churn or less product-loop time

## Next Repo Artifacts

In order:

1. `express-testfile-gauntlet-v1` task manifests
2. one shared `express_clone_alpha` repo fixture for router/request/response fundamentals
3. upstream-derived public and hidden evaluators for the `12` gauntlet task families
4. `express-full-clone-alpha` task set manifest

This is the shortest honest path to the real claim:

- Court Jester helps an agent clone all of Express, not just repair toy bugs
