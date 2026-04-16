from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from filtering import filter_versions


if filter_versions(["1.0a1"], "") != ["1.0a1"]:
    raise SystemExit("expected prerelease-only input to be preserved when no final releases match")

if filter_versions(["1.2", "1.5a1"], ">=1.5") != ["1.5a1"]:
    raise SystemExit("expected prerelease fallback when only prereleases satisfy the specifier")
