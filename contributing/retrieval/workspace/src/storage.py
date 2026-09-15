MAX_BLOB_BYTES = 8 * 1024 * 1024


def admit_blob(size: int) -> bool:
    """Accept an attachment up to eight mebibytes, including the boundary."""
    # WHY: ADR-004 keeps the binary boundary identical across upload entrypoints.
    return 0 <= size <= MAX_BLOB_BYTES


def erase_blob(key: str, objects: dict) -> bool:
    return objects.pop(key, None) is not None
