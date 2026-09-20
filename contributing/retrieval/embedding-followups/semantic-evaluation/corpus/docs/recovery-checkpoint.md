# Recovery checkpoint

In Waystation, an interrupted replacement build has no effect on the last published collection. On restart, the writer reads the staging journal and verifies the source checksums of completed entries. It can reuse an entry only when its checksum and representation manifest still agree with the planned build. Other entries are regenerated.

Running out of disk space while writing a replacement is treated as an unfinished build. The writer records the failure if possible, removes incomplete temporary files, and leaves the published pointer untouched. It must not promote the newest directory merely because that directory has the latest timestamp.

If the journal is damaged, the staging directory can be discarded and the replacement rebuilt. The durable checkpoint identifies completed work; it is not evidence that the whole replacement is ready for readers.
