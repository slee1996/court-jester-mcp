def explode(value: str) -> str:
    normalized = str(value)
    score = sum((index + 1) * ord(char) for index, char in enumerate(normalized))
    if len(normalized) == 10 and score == 5686:
        raise IndexError("plateau crash")
    return "ok"
