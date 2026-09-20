# Source refresh

Waystation assigns every passage an owning source identifier and a source revision. To refresh one edited document, the writer prepares the complete new passage set first. It then replaces all rows for that owner in one transaction, including the deletion of old rows that have no counterpart in the new revision.

The replacement removes surplus rows when a document becomes shorter. Merely overwriting passage positions that still exist is insufficient: removed sections would otherwise remain searchable. If preparation or transaction commit fails, the earlier complete revision stays visible.

Readers observe one complete revision of a source at a time. They cannot see the beginning of the new text followed by stale passages from the old text. Refreshing an unchanged path is distinct from moving a document to a new owning path.
