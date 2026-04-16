Express clone fresh-repo workspace map: response headers

Goal:
- Build response header and status helpers from the visible public spec.
- Focus on header composition, status helpers, and response metadata.

Primary files:
- `lib/http.ts`: response helpers such as `location`, `links`, `vary`, and `sendStatus`.

Visible spec surface:
- `tests/public_spec.ts`: the public behavior spec for response helpers.
- `tests/harness.ts`: request/response harness for the public spec.

Suggested build order:
- Encode `Location` correctly.
- Append `Link` header values across repeated calls.
- Leave `sendStatus` and `vary` edge behavior for deeper checks.
