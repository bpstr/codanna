# Multilingual retrieval

In the synthetic Waystation collection, English documents can answer a question written in Spanish or Hungarian when the selected multilingual encoder places those languages in a shared semantic space. Both document and question representations must come from the same compatible encoder configuration.

The question uses the encoder's query role and the stored passage uses its document role. Language detection is diagnostic information; it does not filter out sources written in a different language. Keyword overlap is not required for a passage to express the requested idea.

A translated question can help diagnose a weak cross-language match, but translating every document is not a prerequisite of this design. The original source language and citation remain available. These are fictional design notes, not a measured claim about the quality of any particular encoder.
