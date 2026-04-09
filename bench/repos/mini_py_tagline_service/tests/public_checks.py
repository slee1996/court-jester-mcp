from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from tagline import primary_tagline

assert primary_tagline({"segments": [" Launch ", "Ignore me"]}) == "Launch"
assert primary_tagline({"segments": ["Focus"]}) == "Focus"
