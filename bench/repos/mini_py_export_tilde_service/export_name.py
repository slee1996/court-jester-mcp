def export_filename(title: str) -> str:
    ascii_title = title.encode("ascii").decode("ascii")
    normalized = ascii_title.strip().lower().replace(" ", "-")
    return normalized + ".csv"
