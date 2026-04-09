from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from query import canonical_query


assert canonical_query({"q": "alpha beta", "page": 2}) == "page=2&q=alpha+beta"
assert canonical_query({"tag": ["pro", "beta"]}) == "tag=pro&tag=beta"
