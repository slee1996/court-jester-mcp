import re


_CORE_RE = re.compile(r"^\s*[vV]?(\d+)(?:\.(\d+))?(?:\.(\d+))?")


def _core_tuple(value: str) -> tuple[int, int, int]:
    match = _CORE_RE.match(value)
    if match is None:
        return (0, 0, 0)
    major, minor, patch = match.groups()
    return (int(major), int(minor or 0), int(patch or 0))


def compare_versions(left: str, right: str) -> int:
    left_core = _core_tuple(left)
    right_core = _core_tuple(right)
    if left_core < right_core:
        return -1
    if left_core > right_core:
        return 1
    return 0
