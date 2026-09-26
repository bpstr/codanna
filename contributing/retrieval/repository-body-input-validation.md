# Implementation inputs on the frozen repository tasks

This #60 follow-up qualifies #55's input-capture and persistence mechanics
against the original #58 source owners. It does not run an embedding model,
change default policy, evaluate semantic relevance, or rebuild a real index.

## What was repaired

The latest #55 runtime did not compile in its contract workflow. The test module
was missing `SemanticSearchRequest`, and the segment-checkpoint writer formatted
a SHA-256 output with an unsupported `LowerHex` implementation. The import is
added and the existing `hex::encode` implementation produces the same intended
lowercase hexadecimal digest. Checkpoint/segment lifecycle tests now execute.

After those repairs the existing disabled-semantic fallback test exposed a
separate response mismatch. A successful no-op preparation could reach the
`has_semantic_search == false` branch, which still instructed the caller that
an index rebuild was needed. Both semantic handlers now explicitly explain
that no rebuild was attempted and name `search_symbols` / `search_context` as
lexical alternatives. Enabling semantic indexing remains a separate explicit
operation. The new handler fixture confirms that no semantic directory appears.

Strict lint also found two key-only map iterations and a test settings
initializer. Those are repaired without suppressions or changes to input,
vector, source, memory or provider budgets.

## Exact input evidence

The oracle and five `.rs.fixture` source files are byte-identical to #58.
The test runs the real native Rust parse/capture stage; it never constructs an
embedding backend. The parser capture flag is enabled only to exercise that
stage. Every retained body excerpt is compared against the original source
bytes at its recorded byte range. Symbol and file byte budgets are asserted.

| Evidence | Observed |
| --- | ---: |
| Validated implementation-owner labels | 10 |
| Owners eligible under comment-only policy | 6 |
| Owners with verified body excerpts under the opt-in policy | 10 |
| Retained owner input bytes, including headers | 19,244 |
| Embedding provider requests in these tests | 0 |
| Token estimate / semantic retrieval quality | Not measured |

The four newly representable undocumented owners are recall `capture`, recall
`render`, embedding-cache `load`, and embedding-cache `save`. Default policy is
still `DocComment`; a separate test verifies that ordinary parsing does not
capture body representations under that default.

### Coverage is not quality

Simple query-word overlap is logged only as diagnostic input evidence. Some
operational questions gain words that were absent from comments. The `capture`
and cache `save` paraphrases still have zero measured word overlap even after
body capture. A bounded header can also omit words from a long documentation
comment; the `scope_for_root` paraphrase loses one word in this simple overlap
comparison. The test deliberately does not assert that more input always means
better relevance.

The 19,244 bytes are not tokenizer output or a projected bill. They cover ten
selected owners, not every eligible workspace symbol. The input-budget
partitioning/backend identity and actual model retrieval must be evaluated
separately before a paid rebuild. No expansion is enabled in production here.

## Executed verification

Original [#55 run 35713773661](https://github.com/bpstr/codanna/actions/runs/35713773661)
stopped at the two compilation errors; it is not a semantic-quality failure.

The first repair run, [35742963664](https://github.com/bpstr/codanna/actions/runs/35742963664),
passed 26 input/persistence/coverage tests, then failed the disabled-semantic
message contract. A following run passed all 28 selected tests but stopped at
three strict lint findings. Those failures remain distinct from passing checks.

[Run 35745153839](https://github.com/bpstr/codanna/actions/runs/35745153839),
starting at `5c8ec911a0e49f5494514bea81c3e49e486376c9`, validates pinned base blob
identities and tests the exact repaired runtime/test blobs committed in this
batch. **28 tests passed**, plus strict all-target/all-feature Clippy:

- two frozen owner-input/default-policy tests;
- one new two-handler fallback/no-semantic-directory test;
- sixteen existing representation, segmentation and parent/vector lifecycle tests;
- five semantic journal checkpoint/delta/tombstone tests;
- three semantic-coverage/metadata tests;
- one existing unavailable-semantic fallback test.

Formatting was applied before that run. The retained final workflow runs the
committed source directly, checks formatting/Clippy, and refuses zero-test
filtered selections. Its final-head outcome is separate from staged evidence.
The temporary handoff workflow is removed; final verification is read-only.

Oracle SHA-256 remains
`129a2c11790e6981134c5ab4d727128b4c27f5f1a1e467b10ee00628c3c90fd6`.

| Verified file | SHA-256 |
| --- | --- |
| `src/semantic/journal.rs` | `c6994abf122355be8968409f0bd8076444811b0b45847bf3e7f4a7104e28c485` |
| `src/indexing/facade_retrieval_tests.rs` | `3a71dce3cf4fb24413e2630b0f3a73122a13834b64a2c5861a7bdd787e993539` |
| `src/mcp/tools/search.rs` | `79a11801279c46b5c7e1afcef980cf7c2ccdf016db5cda0eb8c300053b31d59d` |
| `src/semantic/simple.rs` | `c61fc343cfcaee370cc421210533f5f777955f36a56104e4eed242d8df5c6bcb` |
| `src/symbol_representation.rs` | `8a0084c95df7dd40634c2a7b666ded82115d13cb1d6e4c712b7d144e1c519e5e` |
| `tests/repository_body_inputs.rs` | `4f6124c7d8239d6565df9f905d66b63b8fd454138f583625c04ba2ddae026764` |
| `tests/semantic_fallback_contracts.rs` | `8c1ff558b5299165c795005257431ee10319ee64533c08271ba5a239d1c742ad` |

## Remaining integration

#59's lexical change is independently measured at 8/20, not the original quality
target. #56 needs actual additional-source evidence, not only rank fusion over
the same lexical candidates. These stacked branches, the body-aware cost
preflight, independent relevance labels and controlled model/latency experiments
still need combined qualification. This PR does not approve a paid reindex or
claim that source capture solves paraphrase retrieval.
