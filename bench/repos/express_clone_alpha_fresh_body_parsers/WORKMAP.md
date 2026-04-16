Express clone fresh-repo workspace map: body parsers

Goal:
- Build request body parsing from the visible public spec.
- Focus on JSON and urlencoded parsing first, then broader body-wrapper behavior.

Primary files:
- `index.ts`: wrapper middleware factories (`json`, `urlencoded`, `text`, `raw`).
- `lib/query.ts`: query-string parsing for extended urlencoded bodies.
- `lib/http.ts`: request body helpers.

Visible spec surface:
- `tests/public_spec.ts`: the public behavior spec for body parsing.
- `tests/harness.ts`: request/response harness for the public spec.

Suggested build order:
- Get JSON parsing working.
- Implement extended urlencoded nesting in `lib/query.ts`.
- Leave text/raw edge handling for verifier follow-up.
