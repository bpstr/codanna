from typing import cast
from protocol import RunStore as Store

def load(value: object):
    return cast(Store, value)

def shadow(Store, value):
    return cast(Store, value)
