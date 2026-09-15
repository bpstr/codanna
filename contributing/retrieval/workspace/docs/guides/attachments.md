# Sending attachments

Use the [current policy](../ADR-004.md#binary-boundary), not the archived prototype.
A 413 response means the byte boundary was exceeded. The browser and server both
check the limit; a mobile-only rejection may indicate the old client copy.
The following source link has a target line range: [guard](../../src/storage.py#L4-L7).
