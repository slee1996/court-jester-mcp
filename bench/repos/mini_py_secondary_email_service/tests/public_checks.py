from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from contact import secondary_support_email

assert (
    secondary_support_email(
        {"contacts": {"emails": ["owner@example.com", " Support@Example.com "]}}
    )
    == "support@example.com"
)
