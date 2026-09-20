# Alert digests

The Waystation notification screen calls its email setting "file changes batching." It collects repeated file changes and search index refresh notices into a digest. A quiet-period timer groups related notices, and a maximum waiting interval ensures that a digest is eventually delivered during sustained editing.

This batching rule controls when people receive a summary. Each notice is produced after the search index has already been refreshed. Turning digest batching off sends one notification per completed refresh; it does not make source updates run earlier or later.

Grouping uses the recipient and project rather than the owning source path. Deleting a notice from the pending digest does not remove indexed passages. The email setting and the writer's event-coalescing policy have separate timers even though the interface uses similar words for both.
