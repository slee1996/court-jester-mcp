Express clone fresh-repo workspace map

Goal:
- Build the Express clone from the visible public spec.
- This repo intentionally starts as a bare scaffold with API placeholders, not a prebuilt partial implementation.

Primary files:
- `index.ts`: exports the Express factory plus wrapper middleware helpers.
- `lib/router.ts`: router/app scaffolding and dispatch flow.
- `lib/http.ts`: request/response helpers and response methods.
- `lib/query.ts`: query-string parsing.
- `lib/types.ts`: shared TypeScript types.

Visible spec surface:
- `tests/public_clone_alpha_monolith.ts`: the public behavior spec.
- `tests/harness.ts`: request/response harness for the public spec.

Static assets:
- `static/`: files used by `express.static` checks.

Suggested build order:
- Get the core router/app flow working in `lib/router.ts`.
- Add request and response behavior in `lib/http.ts`.
- Fill in wrapper middleware and static serving in `index.ts`.
- Finish nested query parsing in `lib/query.ts`.
