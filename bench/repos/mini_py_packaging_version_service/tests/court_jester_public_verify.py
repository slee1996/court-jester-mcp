from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from versioning import compare_versions


if compare_versions("1.0rc1", "1.0") >= 0:
    raise SystemExit("expected rc release to sort before final release")

if compare_versions("1.0", "1.0.post1") >= 0:
    raise SystemExit("expected post release to sort after final release")
