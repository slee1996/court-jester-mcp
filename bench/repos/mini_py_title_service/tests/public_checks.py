from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from titles import primary_title

assert primary_title(["Welcome"]) == "Welcome"
assert primary_title(["  News "]) == "News"
