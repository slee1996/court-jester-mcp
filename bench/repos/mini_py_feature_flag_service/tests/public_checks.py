from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from flags import beta_checkout_enabled


assert beta_checkout_enabled(None) is True
assert beta_checkout_enabled({"flags": {"beta_checkout": True}}) is True
