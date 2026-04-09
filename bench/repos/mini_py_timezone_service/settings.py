def preferred_timezone(profile: dict | None) -> str:
    return profile["preferences"]["timezone"].strip()
