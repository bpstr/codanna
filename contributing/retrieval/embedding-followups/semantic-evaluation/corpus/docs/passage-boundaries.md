# Passage boundaries

Waystation's document segmenter first recognizes headings, paragraphs, lists, and tables. It keeps a short table together with its column headings so that a retrieved cell retains its meaning. It also carries the enclosing section heading into the text used to encode each passage.

Ordinary prose is divided at paragraph boundaries before smaller sentence boundaries are considered. A passage can repeat a small amount of neighboring prose as context, but repeated context never substitutes for the source's section structure. A long table is split into row groups that each repeat the relevant column headings.

A fixed character count alone is insufficient: cutting halfway through a row or separating a heading from the statements it qualifies creates fragments that are difficult to interpret. Each segment retains its owning source and its original location for citations.
