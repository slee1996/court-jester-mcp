# Express Full Clone Benchmark Plan

As of April 12, 2026, the right way to interpret "clone all of Express" is:

- implement a repo that exposes the same externally visible Express API surface we choose to support
- evaluate it against the upstream `expressjs/express` test corpus
- treat full-clone success as passing the entire selected upstream test surface, not just a handful of toy tasks

This document defines that benchmark target and the execution plan.

## Thesis

The benchmark should prove that Court Jester helps an agent build a real product-sized framework, not just patch micro-bugs.

For Express, that means:

- router and middleware semantics
- request and response helpers
- body parser wrapper behavior
- static file and rendering helpers
- config/locals/app-level behavior
- regressions already encoded by upstream tests

The benchmark should still answer the product question:

- does `repair-loop-verify-only` help more than tests-only repair loops?
- does it help with acceptable time overhead?
- does it reduce search churn during repair?

## Definition Of Done

We should be explicit about what counts as "all of Express."

`express-full-clone-v1` means:

- target source of truth: upstream `expressjs/express` repository test corpus
- target behavior: pass the selected upstream test files without modifying the tests
- target repo shape: one clone repo fixture, not disconnected toy tasks
- target score: upstream-style public and hidden test subsets plus Court Jester in-loop repair

The final bar is not "agent wrote something framework-like." It is:

- the clone passes the full benchmarked Express test surface

## What Counts As In Scope

In scope for `v1`:

- app lifecycle and routing
- router and route behavior
- middleware ordering and error propagation
- request helper behavior
- response helper behavior
- Express-owned wrapper APIs such as `express.json`, `express.urlencoded`, `express.static`
- regression behaviors already encoded in upstream tests

Out of scope for `v1`:

- middleware in separate repos outside the Express repo unless Express's own test suite directly exercises that surface
- performance benchmarking as a pass/fail gate
- ecosystem compatibility beyond what upstream Express tests assert

## Upstream Surface Inventory

This is the current upstream `test/` surface from `expressjs/express` that matters for the full-clone benchmark.

Top-level support dirs:

- `test/acceptance`
- `test/fixtures`
- `test/support`

Core routing / app / middleware:

- `test/Route.js`
- `test/Router.js`
- `test/app.all.js`
- `test/app.engine.js`
- `test/app.head.js`
- `test/app.js`
- `test/app.listen.js`
- `test/app.locals.js`
- `test/app.options.js`
- `test/app.param.js`
- `test/app.render.js`
- `test/app.request.js`
- `test/app.response.js`
- `test/app.route.js`
- `test/app.router.js`
- `test/app.routes.error.js`
- `test/app.use.js`
- `test/config.js`
- `test/exports.js`
- `test/middleware.basic.js`
- `test/regression.js`
- `test/utils.js`

Express wrapper APIs:

- `test/express.json.js`
- `test/express.raw.js`
- `test/express.static.js`
- `test/express.text.js`
- `test/express.urlencoded.js`

Request API:

- `test/req.accepts.js`
- `test/req.acceptsCharsets.js`
- `test/req.acceptsEncodings.js`
- `test/req.acceptsLanguages.js`
- `test/req.baseUrl.js`
- `test/req.fresh.js`
- `test/req.get.js`
- `test/req.host.js`
- `test/req.hostname.js`
- `test/req.ip.js`
- `test/req.ips.js`
- `test/req.is.js`
- `test/req.path.js`
- `test/req.protocol.js`
- `test/req.query.js`
- `test/req.range.js`
- `test/req.route.js`
- `test/req.secure.js`
- `test/req.signedCookies.js`
- `test/req.stale.js`
- `test/req.subdomains.js`
- `test/req.xhr.js`

Response API:

- `test/res.append.js`
- `test/res.attachment.js`
- `test/res.clearCookie.js`
- `test/res.cookie.js`
- `test/res.download.js`
- `test/res.format.js`
- `test/res.get.js`
- `test/res.json.js`
- `test/res.jsonp.js`
- `test/res.links.js`
- `test/res.locals.js`
- `test/res.location.js`
- `test/res.redirect.js`
- `test/res.render.js`
- `test/res.send.js`
- `test/res.sendFile.js`
- `test/res.sendStatus.js`
- `test/res.set.js`
- `test/res.status.js`
- `test/res.type.js`
- `test/res.vary.js`

## Benchmark Architecture

We should not jump directly from today's micro-fixtures to "one giant task that reimplements Express from scratch."

The right architecture has three levels.

### Level 1: Express Testfile Gauntlet

One benchmark task corresponds to one upstream test file or one tightly coupled pair of files.

Why:

- clean provenance
- clear public/hidden split
- precise failure attribution
- lets us learn which parts of Express Court Jester helps with

This is the immediate build target.

### Level 2: Express Subsystem Slice

A single repo fixture is judged against a bundle of upstream test files from one coherent subsystem.

Examples:

- router core
- request helpers
- response helpers
- body wrapper APIs

Why:

- moves us closer to real product work
- still keeps debugging tractable

### Level 3: Express Full Clone Repo Benchmark

One repo fixture, one agent task, broad upstream test suite.

This is the real "clone all of Express" proof.

Success means:

- agent modifies the clone repo
- public upstream subset passes
- hidden upstream subset passes
- Court Jester helps the repair loop reach parity faster or more reliably

This is the headline destination, not the first artifact we should build.

## Recommended Rollout

### Phase 1: Gauntlet

Build `express-testfile-gauntlet-v1`.

Target:

- `20-30` tasks
- all sourced from upstream Express test files
- TS/JS only
- one fixture repo per task or per tightly scoped family

Policy matrix:

- `baseline`
- `public-repair-1`
- `repair-loop-verify-only`
- `public-repair-2`
- `repair-loop-verify-only-2`

This phase tells us:

- where Court Jester helps on real framework semantics
- whether tests-only repair actually fires
- which Express surfaces are hardest for the verifier

### Phase 2: Subsystem Clones

Build these subsystem benchmarks:

- `express-router-core-v1`
- `express-request-surface-v1`
- `express-response-surface-v1`
- `express-body-and-static-v1`

Each subsystem uses one clone repo and a bundle of upstream tests.

This phase tells us:

- whether Court Jester still helps when the work becomes repo-shaped instead of single-file-shaped

### Phase 3: Full Clone

Build `express-full-clone-v1`.

This is one repo fixture plus a substantial upstream test suite. The final milestone can be expanded to full coverage, but the first full-clone benchmark should still be staged:

- public suite: broad visible subset of upstream tests
- hidden suite: withheld upstream tests from the same files or neighboring files
- Court Jester: in-loop verifier against the clone repo

## Full Clone Milestone Definition

The final milestone should be judged against the entire selected upstream test surface, not a bespoke evaluator.

Recommended milestone ladder:

1. `express-full-clone-alpha`
   - router + app + middleware + request/response helpers
   - excludes rendering and some file-serving edges

2. `express-full-clone-beta`
   - adds wrapper APIs, rendering helpers, static/file helpers, and broader regressions

3. `express-full-clone-v1`
   - passes the complete benchmarked upstream surface we commit to in this repo

## Public / Hidden Split Strategy

For Express, we should not hide entire files blindly. That makes the benchmark hard to interpret.

Use this split:

- public:
  - one or more canonical cases from each upstream test file
  - enough to trigger tests-only repair on some tasks
- hidden:
  - deeper edge cases from the same file
  - neighboring regression cases
  - ordering, precedence, and error-path behavior that plausible clones often miss

Court Jester should compete against:

- no repair
- tests-only repair

Hidden tests remain final-score only.

## What Will Make This Product-Proving

The benchmark succeeds only if it answers product questions, not just academic ones.

Primary metrics:

- final success rate
- additional successes vs baseline
- product minutes per success
- verify-triggered recovery rate
- public-triggered recovery rate

Mechanism metrics:

- public repairs that actually fired
- verify repairs that actually fired
- repair success after trigger
- trace event count on repair attempts
- search/read/shell command mix on repair attempts
- files touched per repair attempt

The strongest product claim will be:

- on real Express behavior, verify-guided repair beats or matches tests-only repair while requiring less search churn or less time per saved task

## Immediate Build Order

Do not start with the easiest request/response helpers.

Start with the surfaces most likely to be product-proving:

1. router and middleware ordering
2. app/router mounting and `baseUrl` / params interactions
3. error propagation and error middleware
4. `req.query`, `req.get`, and proxy-sensitive request helpers
5. `res.format`, `res.redirect`, `res.send`, `res.json`
6. wrapper APIs: `express.json`, `express.urlencoded`, `express.static`

Reason:

- these are closer to "real framework product work"
- they involve semantic correctness and ordering, not just trivial helper outputs
- they are more likely to surface meaningful verifier utility

## Suggested First Wave Task Families

Wave 1 should be derived from these upstream files:

- `test/Router.js`
- `test/Route.js`
- `test/app.router.js`
- `test/app.use.js`
- `test/app.param.js`
- `test/app.routes.error.js`
- `test/req.baseUrl.js`
- `test/req.query.js`
- `test/res.format.js`
- `test/res.redirect.js`
- `test/res.send.js`
- `test/res.json.js`

That is the best first step toward "all of Express" without losing the benchmark to sheer breadth.

## Hard Constraints

We should be explicit about what would invalidate the benchmark.

Invalid benchmark patterns:

- agent prompt pastes the exact public test source on every task
- public-repair never actually fires
- hidden repairs are allowed in the main comparator
- tasks are sourced from blog posts or toy examples instead of upstream Express behavior
- the full-clone repo uses a radically different API surface than Express itself

## Next Repo Artifacts To Build

In order:

1. [express-testfile-gauntlet-v1-plan.md](/Users/spencerlee/court-jester-mcp/bench/express-testfile-gauntlet-v1-plan.md)
   - complete
   - concrete task ids and public/hidden split per upstream file

2. `express-clone-alpha-pilot`
   - in progress
   - shared repo fixture plus seeded regressions for router/app/request/response semantics

3. `express-router-core-v1`
   - first subsystem clone repo

4. `express-full-clone-v1` fixture skeleton
   - one repo fixture with app/router/request/response modules
   - upstream test harness integration

This plan is deliberately ambitious. The goal is not to stay in micro-benchmark land. The goal is to climb from upstream-derived task files to a full framework clone while preserving causal comparisons between tests-only repair and Court Jester-guided repair.
