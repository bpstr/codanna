# Source relocation

Waystation identifies a source by its normalized path relative to the collection root. The identifier includes the path, even when two files contain identical text. A content digest detects edits but does not establish identity across locations.

Moving a guide therefore produces two changes: remove the old source and ingest the new source. Removal purges all passage rows owned by the former path. Ingestion creates rows associated with the destination path. Both changes must finish before the move is reported as settled.

Updating only the destination can leave the old guide searchable. The repair is to reconcile the set of indexed source paths with the current source inventory and remove owners that no longer exist. Deleting a display label or updating a title alone does not remove the old passage rows.
