Express clone monolith workspace map

Goal:
- Fix the shared clone to satisfy the visible monolith public checks.
- The implementation should behave like Express across routing, middleware, request helpers, and response helpers.

Primary files:
- `index.ts`: thin Express entrypoint. Exports the factory plus wrapper middleware and static serving.
- `lib/router.ts`: routing, app mounting, `Route`, `Router`, param handling, and dispatch flow.
- `lib/http.ts`: request decoration, response helpers, header handling, redirect/format/send/json logic.
- `lib/query.ts`: query parser behavior used by `req.query` and `express.urlencoded({ extended: true })`.

Test surface visible to the agent:
- `tests/public_clone_alpha_monolith.ts`: the only visible public benchmark entrypoint for this monolith task.
- `tests/harness.ts`: request/response harness used by the visible public test.

Static assets:
- `static/`: small files used by `express.static` behavior in the monolith checks.

Where to look first:
- Routing and mounting behavior: `lib/router.ts`.
- Body parsing and wrappers: `index.ts` and `lib/query.ts`.
- Request helpers like `req.get`, `req.protocol`, `req.query`: `lib/http.ts` and `lib/query.ts`.
- Response helpers like `res.location`, `res.links`, `res.vary`, `res.sendStatus`: `lib/http.ts`.
