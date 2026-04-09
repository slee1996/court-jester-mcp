from urllib.parse import quote_plus


def canonical_query(params: dict[str, object]) -> str:
    parts: list[str] = []
    for key in sorted(params):
        value = params[key]
        if value is None:
            continue
        if isinstance(value, list):
            for item in value:
                parts.append(f"{quote_plus(key)}={quote_plus(str(item).strip())}")
        else:
            parts.append(f"{quote_plus(key)}={quote_plus(str(value).strip())}")
    return "&".join(parts)
