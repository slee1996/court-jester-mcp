from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from specifiers import allows


if allows("1.0a1", ">=1.0"):
    raise SystemExit("expected prerelease not to satisfy >=1.0 by default")

if allows("1.5.0", "~=1.4.5"):
    raise SystemExit("expected ~=1.4.5 to exclude 1.5.0")
