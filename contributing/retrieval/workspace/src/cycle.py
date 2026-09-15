def first(value: int) -> int:
    return second(value - 1) if value > 0 else 0


def second(value: int) -> int:
    return first(value - 1) if value > 0 else 0
