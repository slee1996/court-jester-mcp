from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from filtering import filter_versions


assert filter_versions(["1.2", "1.3"], ">=1.3") == ["1.3"]
assert filter_versions(["1.2", "1.5a1"], ">=1.5") == ["1.5a1"]
