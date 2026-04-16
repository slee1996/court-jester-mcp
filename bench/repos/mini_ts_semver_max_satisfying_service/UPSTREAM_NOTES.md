This fixture is derived from `node-semver` max-satisfying and caret-range behavior for a narrow contract.

Primary upstream sources:

- `https://github.com/npm/node-semver`
- `https://github.com/npm/node-semver#readme`
- upstream tests for range matching and max satisfying

The benchmark contract is intentionally narrow:

- `maxSatisfying(versions, rangeText) -> string | null`
- supported ranges in this slice: exact versions and caret ranges
- build metadata is ignored for precedence and canonical output
- prereleases are excluded from normal stable ranges
- zero-major caret ranges are narrower than normal major ranges

This fixture does not attempt to implement the full `semver.maxSatisfying` API.
