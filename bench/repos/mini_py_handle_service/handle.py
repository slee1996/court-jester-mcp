def display_handle(user: dict | None) -> str:
    return user["profile"]["handle"].strip().lower()
