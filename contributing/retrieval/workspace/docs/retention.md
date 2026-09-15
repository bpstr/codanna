# Removing archived objects

A customer may ask for a stored attachment to become inaccessible immediately.
The implementation is `erase_blob`; it removes the object rather than hiding a
list item. [Storage](../src/storage.py) has intentionally little inline prose.
No retention scheduler or automatic grace-period job exists in this workspace.
