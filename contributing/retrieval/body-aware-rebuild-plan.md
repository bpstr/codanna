# Body-aware offline rebuild planning

Follow-up to #49 and #60. This branch starts from #60 at
`c294b30df9b48a8539eb3a5c3ad2bf85822d3144`, and copies the existing planner,
auxiliary CLI and twelve process contracts by Git blob identity from #49 at
`652af96b21899927376dbacacc000ee3a1d485b1`. It does not merge unrelated cache
runtime changes or alter any existing branch.

The baseline planner always counts documentation comments. That cannot predict
the configured `symbol_body_v1` inputs. The repair must reuse the actual
parser capture and `SymbolSource::inputs` segmentation, bind the configured
source policy into cache identity, and distinguish parents from inference inputs.

## Acceptance

- Preserve all twelve existing read-only CLI contracts for comment mode.
- Count undocumented eligible bodies without changing the default policy.
- Compare planned input counts/bytes/cache opportunities with actual runtime
  representation inputs using a local known budget, not a generated oracle.
- Count repeated headers across segments; retained source bytes are separate.
- Keep unknown local tokenizer-dependent segments and cache overlap null.
- Reject incompatible comment/body cache identities; changed source must change
  prepared input reuse, not reuse historical numeric symbol IDs.
- Represent missing capture and rejected input preparation as partial/blocked,
  not successful empty inventories. Preserve existing file/entry/input budgets.
- Prove through the CLI that no model/provider is initialized, no workspace
  file is changed, and no active code index is opened.

No paid inference, new source policy, token-price estimate, production index
rebuild or relevance-quality claim is authorized by this work. Default remains
`doc_comment`. Verified body availability is not semantic retrieval quality.
