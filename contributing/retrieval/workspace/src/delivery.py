def settle_once(envelope: str, seen: set[str]) -> bool:
    """Apply a delivery once; replaying the same envelope must do no work."""
    # WHY: ADR-009 treats a courier retry as the original delivery, not a new one.
    if envelope in seen:
        return False
    seen.add(envelope)
    return True


def receive(envelope: str, seen: set[str]) -> bool:
    return settle_once(envelope, seen)
