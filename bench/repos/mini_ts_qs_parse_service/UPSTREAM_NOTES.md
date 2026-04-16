This fixture is derived from `qs` parse behavior for duplicate keys, arrays, and bracket notation.

Primary upstream sources:

- `https://github.com/ljharb/qs`
- `https://github.com/ljharb/qs/blob/main/README.md`
- upstream parse tests in the `qs` repository

The benchmark contract is intentionally narrow:

- `parseQuery(input) -> object`
- repeated flat keys become arrays
- `a[]=b&a[]=c` becomes `{ a: ["b", "c"] }`
- bracket notation like `filter[city]=Paris` becomes nested objects

This fixture does not attempt to implement the full `qs.parse` option surface.
