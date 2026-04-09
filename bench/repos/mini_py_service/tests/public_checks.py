from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from profile import normalize_display_name


assert normalize_display_name("spencer") == "Spencer"
assert normalize_display_name(" Spence ") == "Spence"
assert normalize_display_name(None) == "Anonymous"
