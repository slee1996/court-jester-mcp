This fixture is derived from `qs.stringify` behavior for nested objects, nested arrays, and empty/nullish values.

Primary upstream sources:

- `https://github.com/ljharb/qs`
- `https://github.com/ljharb/qs/blob/main/README.md`
- upstream stringify tests in the `qs` repository

The benchmark contract is intentionally narrow:

- `stringifyQuery(input) -> string`
- repeated top-level arrays use repeated keys
- nested object arrays use `[]` suffixes
- nested objects use bracket notation
- nullish values are skipped
- empty strings are preserved

This fixture does not attempt to implement the full `qs.stringify` option surface.
