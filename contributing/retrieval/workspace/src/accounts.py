def validate(value: str) -> bool:
    """Check an account contact address, not an image or attachment size."""
    return "@" in value
