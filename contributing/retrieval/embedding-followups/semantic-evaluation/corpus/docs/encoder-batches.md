# Encoder batches

Waystation tracks two separate constraints before submitting passages to its encoder: the number of items in a batch and the token allowance for an individual item. A small batch can still contain one passage that exceeds the encoder's input capacity.

The input planner measures each passage with the configured tokenizer. An oversized passage is divided into smaller units at suitable text boundaries before encoding. The resulting units retain source ownership and locations, and required document-role text is included in the token calculation. Silent truncation is prohibited because it would make the unseen tail of a document impossible to retrieve.

Reducing the batch's item count addresses aggregate workload, not an individual overlong input. Increasing the number of requested search results is also unrelated to this limit: that setting takes effect after vectors have been produced and stored.
