# Result window

Waystation distinguishes the requested result limit, the effective result limit, and the number of matches returned. A positive request is capped at forty. A nonpositive request uses the default of eight. The effective limit is an upper bound, not a promise that the response contains that many rows.

Access filtering, duplicate removal, and the minimum similarity threshold can each reduce the returned count. The reader never pads a short response with unrelated passages. A request for twenty results may therefore return seven eligible matches without an error.

Diagnostics report all three numbers so an operator can tell a cap from a shortage of qualifying passages. Increasing a display panel's height or showing more highlighted words does not change the effective result limit or the eligibility rules.
