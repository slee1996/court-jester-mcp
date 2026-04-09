from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from support import support_email_domain

assert support_email_domain({"contacts": {"support_email": "ops@example.com"}}) == "example.com"
assert support_email_domain({"contacts": {"support_email": "HELP@Travel.test"}}) == "travel.test"
