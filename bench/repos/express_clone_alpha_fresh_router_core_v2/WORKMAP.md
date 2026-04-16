Express clone fresh-repo workspace map: router core

Goal:
- Build the router and app dispatch core from the visible public spec.
- Focus on mounting, route dispatch, and app.param behavior before deeper path handling.

Primary files:
- `index.ts`: exports the Express factory plus wrapper middleware helpers.
- `lib/router.ts`: router/app scaffolding and dispatch flow.
- `lib/http.ts`: request decoration used by routing behavior.

Visible spec surface:
- `tests/public_spec.ts`: the public behavior spec for router core.
- `tests/harness.ts`: request/response harness for the public spec.

Suggested build order:
- Make mounted child apps work at their mount path.
- Make `Route(...).all(...)` dispatch correctly.
- Make `app.param(...)` callbacks run per matching request parameter.
- Preserve room for deeper method-rewrite and mounted-path details in follow-up checks.
