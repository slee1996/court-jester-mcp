def primary_tagline(profile: dict | None) -> str:
    return profile["segments"][0].strip()
