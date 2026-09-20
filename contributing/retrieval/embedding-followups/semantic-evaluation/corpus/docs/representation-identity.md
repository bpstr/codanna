# Representation identity

Waystation records the encoder name, its pinned revision, vector width, document and query role prefixes, and normalization rule in a representation manifest. All of these fields contribute to the identity used to namespace stored vectors and reusable encoding results.

Changing an encoder or any of those settings requires a fresh namespace and a replacement encoding of the corpus. Equal vector widths do not imply compatible coordinate systems. Vectors from different representation identities must never be searched together, even if their lengths match.

Before serving a collection, the reader compares its query encoder configuration with the stored manifest and rejects a mismatch. The manifest defines compatibility; the collection publication procedure defines when readers begin using the replacement. Updating a model name in a settings file does not transform previously stored vectors.
