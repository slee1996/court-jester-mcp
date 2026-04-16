This fixture is derived from `qs.parse` behavior for nested objects, nested arrays, and duplicate keys.

Primary upstream sources:

- `https://github.com/ljharb/qs`
- `https://github.com/ljharb/qs/blob/main/README.md`
- upstream parse tests in the `qs` repository

The benchmark contract is intentionally narrow:

- `parseQuery(input) -> object`
- repeated flat keys become arrays
- bracket notation builds nested objects
- `filter[tags][]=pro&filter[tags][]=beta` becomes `{ filter: { tags: ["pro", "beta"] } }`
- repeated nested scalar keys become arrays

This fixture does not attempt to implement the full `qs.parse` option surface.
