def legacy_upload_limit(size: int) -> bool:
    """Archived sixteen-mebibyte upload prototype; never an active policy."""
    return size <= 16 * 1024 * 1024
