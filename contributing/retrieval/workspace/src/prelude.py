# WHY: ADR-004 applies before allocation, even when the check has no docstring.
def precheck(size: int) -> bool:
    return size <= 8 * 1024 * 1024
