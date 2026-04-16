# Express Clone Alpha Status

Status as of April 12, 2026.

This is the first real benchmark lane aimed at the larger claim:

- Court Jester can help an agent build a framework-sized product, not just patch toy functions
- and, more specifically, whether agents are more successful repairing a large shared library/framework clone with Court Jester than without it

The current artifact is not full Express yet. It is a shared `express_clone_alpha` fixture for the slice tasks plus a dedicated `express_clone_alpha_monolith` fixture for the large-task benchmark.

There are now four distinct benchmark lanes on top of these fixtures:

- `express-clone-alpha-pilot`
  - many seeded single-surface tasks in the same shared repo
- `express-clone-alpha-monolith`
  - one large seeded task that asks the agent to repair a broad Express alpha slice in one shot using a dedicated fixture with a reduced visible test surface
- `express-clone-alpha-fresh-spec`
  - one broad fresh-repo build-from-spec task using only visible public tests plus isolated verify/hidden evaluators
- `express-clone-alpha-fresh-chunks`
  - earlier fresh-repo chunk suite that was useful for finding the next bottlenecks but still too wide/too fuzzy in places
- `express-clone-alpha-fresh-chunks-v4`
  - current tuned fresh-repo chunk suite with tests-only verify, aligned request/response semantics, and narrower router/static slices

## What Exists Now

- shared slice-task fixture repo:
  - `bench/repos/express_clone_alpha`
- dedicated monolith fixture repo:
  - `bench/repos/express_clone_alpha_monolith`
- broad fresh-spec fixture repo:
  - `bench/repos/express_clone_alpha_fresh_spec`
- chunked fresh-spec fixture repos:
  - `bench/repos/express_clone_alpha_fresh_router_dispatch`
  - `bench/repos/express_clone_alpha_fresh_urlencoded`
  - `bench/repos/express_clone_alpha_fresh_request_meta_v2`
  - `bench/repos/express_clone_alpha_fresh_response_headers_v2`
  - `bench/repos/express_clone_alpha_fresh_static_file_v2`
- seeded pilot task set:
  - `bench/task_sets/express-clone-alpha-pilot.json`
- seeded monolith task set:
  - `bench/task_sets/express-clone-alpha-monolith.json`
- broad fresh-spec task set:
  - `bench/task_sets/express-clone-alpha-fresh-spec.json`
- chunked fresh-spec task set:
  - `bench/task_sets/express-clone-alpha-fresh-chunks-v4.json`
- fresh ladder task set:
  - `bench/task_sets/express-clone-alpha-fresh-ladder.json`
- task count:
  - `21`
- covered surfaces:
  - route dispatch
  - child-app mounting
  - param routing
  - router-as-route-callback param isolation
  - route-level error propagation
  - `express.json`
  - `express.raw`
  - `express.text`
  - `express.urlencoded`
  - `req.baseUrl`
  - `req.get`
  - `req.protocol`
  - `req.query`
  - `res.links`
  - `res.location`
  - `res.send`
  - `res.json`
  - `res.sendStatus`
  - `res.vary`
  - `res.format`
  - `res.redirect`
  - `express.static`

Monolith task:

- `bench/tasks/ts-express-clone-alpha-monolith.json`
- seeded regressions across routing, wrappers, request metadata, header helpers, response APIs, and static fallthrough behavior

## Local Known-Good Validation

The clean slice-task fixture currently passes all checked-in local public and verify test files when run directly.

Validated files:

- `tests/public_*.ts`
- `tests/verify_*.ts`

Hidden tests are now isolated outside the fixture under:

- `bench/hidden_assets/express_clone_alpha/tests`

They should be validated through `bench/evaluators/ts_workspace_test.py`, which materializes them only at scoring time. That keeps the hidden suite out of the agent-visible workspace.

This establishes that the shared fixture itself is viable as a known-good Express-clone alpha without leaking hidden tests to the agent.

The large shared-repo monolith also validates under the same isolation model:

- `tests/public_clone_alpha_monolith.ts`
- verifier suite materialized from `bench/verify_assets/express_clone_alpha_monolith/tests/verify_clone_alpha_monolith.ts`
- hidden monolith validation via `bench/evaluators/ts_workspace_test.py`

For the monolith fixture, the agent-visible workspace now contains only:

- `tests/public_clone_alpha_monolith.ts`
- `tests/harness.ts`

Verifier and hidden suites are both kept out of the workspace and materialized only during evaluation.

## Seeded Bug Validation

Seeded `noop` baseline run:

- output dir: `/tmp/court-jester-express-alpha-noop-v8`
- result: superseded by `/tmp/court-jester-express-alpha-noop-v10`

Current widened seeded `noop` baseline:

- output dir: `/tmp/court-jester-express-alpha-noop-v10`
- result: `0 / 21` success

Failure shape:

- `19 / 21` fail publicly
- `2 / 21` are hidden-only

That is the intended shape for a useful public-vs-verify comparator:

- public-repair should actually fire on most tasks
- at least one task still requires hidden/verify strength

The hidden-only task today is:

- `ts-express-app-param-routing`

Monolith seeded `noop` baseline:

- output dir: `/tmp/court-jester-express-alpha-monolith-noop`
- result: `0 / 1` success
- failure shape:
  - public failure on attempt 1

That is the first direct benchmark in this repo that matches the broader product thesis:

- can an agent repair a large amount of shared framework code more successfully with Court Jester than without it

Fresh chunked `noop` baseline:

- output dir: `/tmp/court-jester-express-fresh-chunks-noop`
- result: `0 / 5` success

Failure shape:

- `5 / 5` fail publicly on attempt 1
- each chunk also has isolated verifier and hidden follow-up checks behind the visible public spec

This is the intended shape for tuning chunk size:

- public-repair is guaranteed to engage on the clean scaffold
- verifier and hidden checks still have room to differentiate partial fixes from deeper semantic parity

Current tuned fresh chunked `noop` baseline:

- output dir: `/tmp/court-jester-express-fresh-chunks-v4-noop2`
- result: `0 / 5` success

Current tuned chunk shape:

- `ts-express-fresh-router-dispatch`
  - public: mounted child-app dispatch plus standalone `Route(...).all(...)`
  - verify/hidden: mounted-path bookkeeping
- `ts-express-fresh-urlencoded-v2`
  - public: one visible extended urlencoded nesting case
  - verify/hidden: deeper array/object nesting
- `ts-express-fresh-request-meta-v2`
  - public: `req.get`, trust proxy, and `req.xhr`
  - verify/hidden: extended query parsing plus forwarded-proto edge cases
- `ts-express-fresh-response-headers-v2`
  - public: `location`, `links`, and `sendStatus(204)` empty-body behavior
  - verify/hidden: `location("back")` plus `vary` canonicalization
- `ts-express-fresh-static-file-v2`
  - public: one visible static-file serving case
  - verify/hidden: content-type semantics for the served file

Framework benchmark note:

- these tuned fresh-spec chunks now use task-level `verify_tests_only`
- that makes Court Jester skip generic execute fuzz and judge only against the authoritative verify spec for these framework slices
- reason: generic helper fuzz was the dominant source of `verify_stronger_than_eval` false positives on repo-shaped Express tasks

## Real Agent Smoke

Initial Codex smoke run:

- output dir: `/tmp/court-jester-express-alpha-codex-smoke`
- task: `ts-express-res-redirect-rendering`
- model: `codex-default`

Observed result so far:

- baseline completed `success`
- Codex repaired the seeded redirect bug in one attempt
- public and hidden checks both passed
- agent trace captured the path through:
  - reading `index.ts`
  - reading public and verify redirect tests
  - running local node/npm checks

The paired `repair-loop-verify-only` cell was still in progress at the time this status note was written.

Current Codex pilot run:

- output dir: `/tmp/court-jester-express-alpha-codex-pilot-v2`
- task set: the earlier widened `13`-task `express-clone-alpha-pilot` snapshot
- model: `codex-default`
- policies:
  - `baseline`
  - `repair-loop-verify-only`

That run is the current product-shaped benchmark continuation while the `15`-task pilot validation is being finalized locally.

Current observed progress on that run:

- first completed cell:
  - `ts-express-app-use-mounting`
  - `codex-default`
  - `baseline`
  - `success`

## Hidden-Test Isolation Fix

Earlier Express benchmark runs were contaminated because hidden `.ts` files lived under `bench/repos/express_clone_alpha/tests`, which meant CLI agents could read them after the fixture was copied into the workspace.

That is now fixed for hidden tests:

- hidden Express tests live under `bench/hidden_assets/express_clone_alpha/tests`
- `bench/evaluators/ts_workspace_test.py` materializes them only during hidden scoring
- the copied agent workspace no longer contains `tests/hidden_*.ts`

The monolith lane now also fixes verifier leakage:

- verifier suites for the monolith fixture live under `bench/verify_assets/express_clone_alpha_monolith/tests`
- `bench/runner.py` materializes the verifier file only for the Court Jester call, then removes it
- the copied monolith workspace no longer contains `tests/verify_*.ts`

Any Express benchmark evidence collected before this isolation change should be treated as invalid for hidden-eval claims.

## Current Limitation

The full `required-final` known-good control is not yet usable on this repo shape.

Current issue:

- Court Jester verify reports `verify_stronger_than_eval` on the aggregate Express alpha known-good control

Interpretation:

- the shared clone fixture is good enough for public/hidden testing
- the current verifier still produces framework-shaped false positives on this repo when aggregated too broadly

That is a verifier limitation, not a fixture correctness failure.

## What Is Still Missing Before We Can Claim "Full Express"

- more upstream Express surfaces:
  - request helpers beyond `req.get`
  - response helpers like `res.status` and `res.type`
  - wrapper APIs like `express.static`, `express.raw`, `express.text`
  - broader app lifecycle/config surfaces
- subsystem bundles:
  - `express-router-core-v1`
  - `express-request-surface-v1`
  - `express-response-surface-v1`
- a broader full-clone fixture and test harness integration against a larger upstream file bundle

So the honest current label is:

- `express_clone_alpha`
- not `express-full-clone-v1`

But the benchmark direction is now correct:

- the pilot suite measures broad framework surface utility
- the monolith suite measures the actual large-shared-code repair question
