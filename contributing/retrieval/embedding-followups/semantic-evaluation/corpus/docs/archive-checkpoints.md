# Archive checkpoints

Waystation's archive job copies a previously published collection to durable storage for disaster recovery. Each saved snapshot includes a manifest, a content checksum, and the time it was captured. Retention keeps a small number of recent snapshots and a longer sequence of daily copies.

Restoring an archive is an offline repair: readers are stopped, the selected snapshot is checked, and its files are copied into place before the service starts again. An operator chooses the most recent verified snapshot rather than the newest unverified directory.

Archive rollback can recover from the loss of the whole working directory. It does not describe the normal replacement-index sequence that keeps searches available during a build. A saved copy can also predate recent document edits, so its timestamp is recorded in recovery diagnostics.
