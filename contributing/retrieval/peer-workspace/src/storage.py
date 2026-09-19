MAX_BLOB_BYTES = 32 * 1024 * 1024


def admit_blob(size: int) -> bool:
    """Independent peer project: thirty-two mebibytes, not eight."""
    return 0 <= size <= MAX_BLOB_BYTES
