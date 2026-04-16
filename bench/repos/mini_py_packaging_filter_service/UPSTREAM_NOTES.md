This fixture is derived from `pypa/packaging` `Specifier.filter(...)` behavior around prerelease fallback.

Primary upstream sources:

- `https://packaging.pypa.io/en/stable/specifiers.html`
- `https://github.com/pypa/packaging`
- upstream tests under `tests/test_specifiers.py`

The benchmark contract is intentionally narrow:

- `filter_versions(candidates, specifier) -> list[str]`
- supported specifiers in this slice: empty string, `>=...`, `<...`, `==...`
- preserve input order
- exclude prereleases when matching final releases exist
- include prereleases when they are the only matching candidates

This fixture does not attempt to implement the full `packaging.specifiers.SpecifierSet` API.
