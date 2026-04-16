Express clone fresh-repo workspace map: request metadata

Goal:
- Build request introspection behavior from the visible public spec.
- Focus on header lookup, trust proxy, and request decoration.

Primary files:
- `lib/http.ts`: request helpers such as `req.get`, protocol, secure, and xhr.
- `lib/query.ts`: query parsing used by `req.query`.
- `lib/router.ts`: app settings flow for query-parser behavior.

Visible spec surface:
- `tests/public_spec.ts`: the public behavior spec for request metadata.
- `tests/harness.ts`: request/response harness for the public spec.

Suggested build order:
- Get `req.get()` working.
- Respect trust proxy when computing protocol and secure.
- Leave deeper query-parser and forwarded-header semantics for follow-up checks.
