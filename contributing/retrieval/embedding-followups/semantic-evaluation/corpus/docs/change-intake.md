# Change intake

Waystation receives file changes from an operating-system watcher. Its batching rule groups events by normalized source path. Each new event for a path replaces the pending event for that path and restarts a short quiet-period timer. When the timer expires, the writer reads the current file once and refreshes the search index from that final state.

A sustained stream cannot postpone work indefinitely: a maximum waiting interval flushes pending paths even when the quiet period has not arrived. If the final state is absent, the flush removes that source's indexed passages. It does not replay every temporary save produced by an editor.

The batch controls actual refresh work. It does not schedule user notifications or combine independent paths into one document. Events that arrive during a flush form the next pending batch.
