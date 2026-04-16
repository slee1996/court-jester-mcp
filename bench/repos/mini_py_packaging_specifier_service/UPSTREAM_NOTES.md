This fixture is derived from `pypa/packaging` specifier-set behavior under PEP 440.

Reference sources used while building this shard:

- `https://github.com/pypa/packaging`
- upstream tests under `tests/test_specifiers.py`

The checked-in public, verify, and hidden cases are a compact original subset derived from upstream expectations for:

- prerelease exclusion by default
- inclusive and exclusive comparison operators
- compatible release (`~=`) semantics

This fixture freezes a narrow benchmark contract around `allows(version, specifier) -> bool`.
