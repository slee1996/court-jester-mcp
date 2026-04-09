from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from settings import preferred_timezone

assert preferred_timezone({"preferences": {"timezone": "UTC"}}) == "UTC"
assert preferred_timezone({"preferences": {"timezone": " America/Denver "}}) == "America/Denver"
