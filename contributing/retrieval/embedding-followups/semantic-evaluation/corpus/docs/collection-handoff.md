# Collection handoff

The fictional Waystation reader keeps serving the published collection while a replacement is assembled in a separate staging directory. Building a replacement does not clear the published data. The writer finishes every source, checks the representation manifest and row counts, and flushes the completed files before publishing their directory through one atomic pointer change.

A request pins the collection pointer when it begins and uses that collection until it finishes. Requests that start after publication select the new collection. Retired files are reclaimed only after the last reader releases them. This ordering lets a long search finish against the earlier collection without observing a mixture of generations.

Publication is the final step. A partially built directory is never selected by readers, even if some of its sources already passed validation.
