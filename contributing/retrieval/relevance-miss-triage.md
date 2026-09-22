# Remaining frozen-query misses after normalized coverage

This is the per-owner diagnosis for #59, using unchanged #58 judgments and the
executed bounded lexical pools. No source comment, query, owner label or provider
policy is changed here. These are source-inspection conclusions, not a newly
measured semantic model or a new test set.

## Four owners still returned only in expanded output

| Owner / question family | Expanded rank after selected coverage | What the stored fields actually support |
| --- | ---: | --- |
| `read_file`, giant-artifact paraphrase | 22 | The doc says it reads a file and computes SHA256. It does not explain the byte-budget behavior requested by the question. Merely being a candidate is not strong task evidence. |
| `pair_relocations`, copied-twins paraphrase | 21 | Documentation describes unique old/new content hashes and multiplicity. The question uses different language. Reordering based on the same literal words cannot supply the missing equivalence. |
| `render`, foreign-recall operational question | 7 | The function has no documentation. Its path identifies recall, and its signature contains workspace. More descriptive test helpers win ahead of the implementation. The recorded test-to-render Calls edge offers a possible related-implementation route. |
| `embedding_batch_size`, nearly-full-GPU paraphrase | 23 | Documentation says inference, headroom, accelerated providers and activation buffers. The question says send fewer inputs and GPU nearly full. This is a terminology gap even though common lexical terms admitted a candidate. |

The expanded positions are coverage-ranked results at limit 200, not raw
Tantivy candidate positions. Three cases above illustrate why the original
six "retrievable but low-ranked" cases should not all be treated as score-weight
problems. Candidate presence and useful owner-level evidence are different.

## Eight queries still absent from the bounded result set

- `is_modified`: both phrasings. The documentation describes precise past mtimes
  and uncertain timestamps; the operational question asks about nanosecond edits,
  while the paraphrase describes two saves before the clock advances.
- `collect_all_files`: unreadable-folder paraphrase. Its operational phrasing is
  now recovered; no synonym list is added to force the second one to match.
- `capture`: both phrasings. It is undocumented; the first operational lexical
  hit `conversation_context` has a resolved indexed Calls edge to `capture`.
- `render`, cache `load`, and cache `save`: one paraphrase each. All three owners
  are undocumented under the old input policy.

Absence at the 200-result budget does not establish absence from the corpus or
from every possible lexical candidate. All owner definitions are present and
validated independently by path/name in the current index.

## What follows from the evidence

1. Keep the two measured operational recoveries without pretending the
   paraphrase problem is fixed. Raw-score-only sorting and file quotas lost a
   known success; they are rejected controls, not additional shipped knobs.
2. Treat graph-related implementation suggestions as separately attributed
   evidence. A test or wrapper calling a function does not make that callee the
   correct owner automatically. Any expansion needs its own candidate budget,
   scope, negative controls and first-five quality comparison.
3. Validate #55's body excerpts against exact source bytes before using a model.
   #60 performs that capture/persistence check on the same owners. Bodies make
   four previously ineligible owners representable, but they do not demonstrate
   that an embedding model will retrieve the questions correctly.
4. Evaluate #56 with genuinely available documents/anchors or semantic inputs.
   Combining lexical ranks alone cannot manufacture missing semantic evidence.
   Keep a no-additional-source control; never create oracle-answer documents and
   call their recovery a production benchmark.

The original 0.90 Hit@5 / 0.75 MRR@5 goal remains unmet. The frozen tasks have
influenced selection and are not an independent holdout. No real reindex or
provider budget is authorized by this diagnostic record.
