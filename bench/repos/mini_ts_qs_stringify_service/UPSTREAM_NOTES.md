This fixture is derived from `qs` stringify behavior for a narrow option set.

Reference sources used while building this shard:

- `https://github.com/ljharb/qs`
- upstream tests under `test/stringify.js`

The checked-in cases are a compact original subset derived from upstream behavior for:

- repeated array entries
- nested object bracket encoding
- null skipping

This fixture freezes a small benchmark contract equivalent to `qs.stringify(value, { arrayFormat: 'repeat', skipNulls: true })` for the supported input shapes.
