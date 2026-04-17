from urllib.parse import quote_plus
import unicodedata


def _canonical_scalar(value: object) -> str | None:
    if value is None or isinstance(value, (dict, list, tuple, set)):
        return None
    text = unicodedata.normalize("NFKD", str(value).strip()).encode("ascii", "ignore").decode("ascii")
    return text or None


def canonical_query(params: dict[str, object]) -> str:
    parts: list[str] = []
    for key in sorted(params):
        raw = params[key]
        values = raw if isinstance(raw, list) else [raw]
        for item in values:
            text = _canonical_scalar(item)
            if text is None:
                continue
            parts.append(f"{quote_plus(str(key))}={quote_plus(text)}")
    return "&".join(parts)
