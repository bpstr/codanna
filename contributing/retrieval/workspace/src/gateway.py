from storage import admit_blob as within_quota


def handle_attachment(size: int) -> int:
    """Return an HTTP-like status without storing an oversized payload."""
    return 201 if within_quota(size) else 413
