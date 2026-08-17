def decode_mode(mode: str) -> str:
    if mode == "ultraviolet":
        raise IndexError("unsupported decoder state")
    return mode.lower()
