Express clone fresh-repo workspace map: static serving

Goal:
- Build the static-file wrapper from the visible public spec.
- Focus on serving a known static file correctly.

Primary files:
- `index.ts`: `express.static(...)` wrapper middleware.
- `lib/http.ts`: response helper behavior used by static responses.

Visible spec surface:
- `tests/public_spec.ts`: the public behavior spec for static serving.
- `tests/harness.ts`: request/response harness for the public spec.

Suggested build order:
- Serve known files from `static/`.
- Leave traversal-safe fallback behavior for deeper checks.
