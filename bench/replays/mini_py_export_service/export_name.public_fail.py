import re
import unicodedata


def export_filename(title: str) -> str:
    normalized = unicodedata.normalize("NFKD", title).encode("ascii", "ignore").decode("ascii")
    slug = re.sub(r"\s+", "_", normalized.strip().lower())
    return slug + ".csv"
