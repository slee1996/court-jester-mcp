This fixture is derived from `pypa/packaging` version-ordering behavior under PEP 440.

Reference sources used while building this shard:

- `https://github.com/pypa/packaging`
- upstream tests under `tests/test_version.py`

The checked-in public, verify, and hidden cases are a compact original subset derived from upstream ordering expectations for:

- dev releases
- alpha/beta/rc prereleases
- final releases
- post releases

This fixture does not attempt to implement the full `packaging.version.Version` API. It freezes a small benchmark contract around `compare_versions(left, right) -> -1|0|1`.
