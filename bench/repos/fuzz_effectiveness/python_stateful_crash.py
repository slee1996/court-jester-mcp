def create_counter():
    calls = 0

    def push(value: int) -> int:
        nonlocal calls
        calls += 1
        if calls == 2:
            raise IndexError("counter second-step crash")
        return value

    return {"push": push}
