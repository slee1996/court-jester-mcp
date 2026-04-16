import re


_CORE_RE = re.compile(r"^\s*[vV]?(\d+)(?:\.(\d+))?(?:\.(\d+))?")


def _normalize(value: str) -> str:
    return value.strip().lstrip("vV")


def _core_tuple(value: str) -> tuple[int, int, int]:
    match = _CORE_RE.match(value)
    if match is None:
        return (0, 0, 0)
    major, minor, patch = match.groups()
    return (int(major), int(minor or 0), int(patch or 0))


def _has_prerelease(value: str) -> bool:
    lowered = value.lower()
    return any(token in lowered for token in ("a", "b", "rc", "dev"))


def _compare_core(left: str, right: str) -> int:
    left_core = _core_tuple(left)
    right_core = _core_tuple(right)
    if left_core < right_core:
        return -1
    if left_core > right_core:
        return 1
    return 0


def _matches(version: str, specifier: str) -> bool:
    specifier = specifier.strip()
    if not specifier:
        return True
    if specifier.startswith(">="):
        return _compare_core(version, specifier[2:]) >= 0
    if specifier.startswith("<"):
        return _compare_core(version, specifier[1:]) < 0
    if specifier.startswith("=="):
        return _normalize(version) == _normalize(specifier[2:])
    return False


def filter_versions(candidates: list[str], specifier: str) -> list[str]:
    matches: list[str] = []
    for candidate in candidates:
        if _has_prerelease(candidate):
            continue
        if _matches(candidate, specifier):
            matches.append(candidate)
    return matches
