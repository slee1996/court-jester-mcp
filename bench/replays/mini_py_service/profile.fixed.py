def normalize_display_name(name: str | None) -> str:
    if name is None:
        return "Anonymous"

    stripped = name.strip()
    if not stripped:
        return "Anonymous"
    return stripped[0].upper() + stripped[1:]
