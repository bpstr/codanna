from delivery import receive
from storage import admit_blob


def test_duplicate_delivery():
    """Exercise `settle_once` through the public receiver."""
    seen = set()
    assert receive("envelope-a", seen)
    assert not receive("envelope-a", seen)


def test_attachment_boundary():
    """Validate the documented `admit_blob` boundary."""
    assert admit_blob(8 * 1024 * 1024)
    assert not admit_blob(8 * 1024 * 1024 + 1)
