from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from billing import billing_country

assert billing_country({"billing": {"country": "us"}}) == "US"
assert billing_country({"billing": {"country": " ca "}}) == "CA"
