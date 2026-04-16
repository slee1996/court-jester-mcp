import re


_CORE_RE = re.compile(r"^\s*[vV]?(\d+)(?:\.(\d+))?(?:\.(\d+))?")


def _core_tuple(value: str) -> tuple[int, int, int]:
    match = _CORE_RE.match(value)
    if match is None:
        return (0, 0, 0)
    major, minor, patch = match.groups()
    return (int(major), int(minor or 0), int(patch or 0))


def _has_prerelease(value: str) -> bool:
    lowered = value.lower()
    return any(token in lowered for token in ("a", "b", "rc", "dev"))


def _compare(left: str, right: str) -> int:
    left_core = _core_tuple(left)
    right_core = _core_tuple(right)
    if left_core < right_core:
        return -1
    if left_core > right_core:
        return 1
    return 0


def allows(version: str, specifier: str) -> bool:
    version = version.strip()
    specifier = specifier.strip()
    if specifier.startswith(">="):
        return _compare(version, specifier[2:]) >= 0
    if specifier.startswith("<"):
        return _compare(version, specifier[1:]) < 0
    if specifier.startswith("=="):
        return _compare(version, specifier[2:]) == 0
    if specifier.startswith("~="):
        base = specifier[2:]
        version_core = _core_tuple(version)
        base_core = _core_tuple(base)
        if _compare(version, base) < 0:
            return False
        return version_core[0] == base_core[0]
    return False
