Express clone fresh-repo workspace map: urlencoded parsing

Goal:
- Build extended urlencoded parsing from the visible public spec.
- Focus on nested form parsing only; the surrounding app scaffolding already exists.

Primary files:
- `index.ts`: wrapper middleware factory entrypoint for `urlencoded`.
- `lib/query.ts`: query-string parsing for extended urlencoded bodies.

Visible spec surface:
- `tests/public_spec.ts`: the public behavior spec for nested urlencoded parsing.
- `tests/harness.ts`: request/response harness for the public spec.

Suggested build order:
- Implement extended urlencoded nesting in `lib/query.ts`.
- Wire `express.urlencoded({ extended: true })` to that parser.
- Preserve room for deeper nested arrays and objects in follow-up checks.
