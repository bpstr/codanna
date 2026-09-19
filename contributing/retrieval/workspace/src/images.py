def validate(value: str) -> bool:
    """Allow supported picture media types before thumbnail decoding."""
    return value in {"image/png", "image/jpeg"}
