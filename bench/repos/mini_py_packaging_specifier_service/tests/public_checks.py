from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from specifiers import allows


assert allows("1.0", ">=1.0") is True
assert allows("2.0.0", "<2.0") is False
assert allows("1.4.5", "~=1.4") is True
