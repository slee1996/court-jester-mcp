from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from versioning import compare_versions


assert compare_versions("1.0a1", "1.0") < 0
assert compare_versions("1.0", "1.0.post1") < 0
assert compare_versions("1.2", "1.10") < 0
