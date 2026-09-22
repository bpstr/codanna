# Body-aware rebuild planning and cache reuse

This #64 integration combines the body-aware planner/representation from #63
(`1662e95e14323dbd197a381ff9789d103df8a538`) with the actual cache-reuse runtime
from #50 (`00f8c7daba1fc1ac683469c7c6a8c214fddd01f0`, including #47). Existing
branches and histories remain intact. Main and upstream have not been modified.

## What changed

### Relative configuration paths identify the same workspace

A bare `.codanna/settings.toml` has an empty lexical parent. The config loader
previously retained that empty path when canonicalization failed, while the
offline planner canonicalized its config argument first. The ordinary CLI and
planner could consequently prepare different path/module headers for the same
body. An otherwise reusable cache appeared to contain no matches.

Only the empty inferred parent is now mapped to `.` before normal path
normalization. Explicit workspace roots and nonempty config parents retain the
existing behavior. Actual CLI tests require planner/rebuild parity and prohibit
the machine-specific workspace root in prepared body inputs.

### Body cache hits survive admission of earlier misses

Comment inputs already use #50's collector-batch lookup and deduplication.
The body path previously processed one parent at a time, allowing early misses
to evict compatible entries needed by later parents in the same collector batch.

The embed stage now retains Arc references to existing matching body-input
vectors before admitting new inputs. It then processes parents with the existing
bounded inference and atomic segment-group replacement. Cached vectors attach to
current symbol IDs and fresh source ranges, never historical numeric mappings.

Identical prepared segments within a parent are generated once and shared, but
their separate source-range records remain. Missing/invalid segment groups still
fail instead of publishing a partially assembled parent.

The cache capacity is not enlarged. Pinning can extend vector lifetimes until
the collector batch ends, and source inputs are prepared twice. This trades
additional segmentation work and bounded transient vector retention for reuse;
it is not a measured RSS or latency improvement. Multiple collector batches,
cache capacity, changed inputs, retries and identity changes can still cost work.

## Executed measurements

[Run 35778083206](https://github.com/bpstr/codanna/actions/runs/35778083206), branch
checkpoint `4ba476ef5fe9aaed332bbbc61328f59894360ab5`, locally combined only the
pinned dependency heads, recorded two before-states, then executed the exact
runtime/test blobs committed with this document. No remote branch was updated
by the verifier. Its local merge checkout was
`1a5ca3dcbacdeca331d573c14c4bf5e35f670089`; post-patch file hashes below identify
the executed candidate rather than claiming the checkout hash alone contains it.

The same four process-fixture bytes produced:

| State | Passed | Failed | Meaning |
| --- | ---: | ---: | --- |
| Original combined runtime | 1 | 3 | Relative-root mismatch plus missing body admission/deduplication. |
| Configuration root repaired only | 2 | 2 | Ordinary warm/edit/delete reuse works; pressure and repeated segments still fail. |
| Full candidate | 4 | 0 | Planner parity, bounded reuse and segment persistence all pass. |

Fixture SHA-256 was unchanged across those three executions:
`d67a20c1c5b904b1b74746ebe6de15f70c59c5fc1348760dc74e6ff1b012b16e`.

| Process scenario | Observed source input items sent |
| --- | --- |
| Two body parents, initial build | 2 |
| Unchanged bodies with shifted current symbol IDs | 0 |
| One implementation body edited | 1 |
| Delete one parent while retaining an unchanged implementation | 0; deleted parent absent after reopen |
| Repeated literal body, seven stored segments | 7 before deduplication; **3 after**, then 0 on warm rebuild |
| 1,280 one-segment parents, 4,096-dimensional vectors | 1,280 initial inputs |
| Same corpus with 1,008 compatible cache entries | 1,280 before admission repair; **272 after** |
| Third same-order pressure rebuild | **272**, retaining all 1,280 parents/vectors |

The 4,096-dimensional fixture reaches the existing 16 MiB logical cache budget
at 1,008 entries. Cache contents continue to be admitted under the existing
policy; they are not frozen for one favorable pass.

Every actual rebuild still made one separately counted remote initialization
probe. The offline planner made zero provider requests and preserved workspace
files, directory membership and modification times. Input-item counts are not
HTTP request counts, billed tokens, prices, or production latency. Fixed vectors
prove reuse and persistence, not semantic relevance.

Comment/body policy switches, changed model/dimension/revision controls and a
corrupt optional accelerator do not become false hits. The original seven #50
CLI cases, including 4,500-comment pressure and repeated-comment deduplication,
remain passing in the combined runtime.

## Verification totals

The candidate run passed **108 tests, zero failed**, followed by formatting and
strict all-target/all-feature Clippy. Nine pre-existing ignored filesystem/CWD
cases remained ignored in the broader filtered selections; no new test was
ignored, retried, or weakened to make the candidate pass.

| Selection | Passed | Existing ignored |
| --- | ---: | ---: |
| Existing + new actual rebuild CLI cases | 11 | 0 |
| Legacy planner CLI | 12 | 0 |
| Body planner CLI | 5 | 0 |
| Planner/runtime input and frozen-source parity | 5 | 0 |
| Representation/segmentation contracts | 16 | 0 |
| Cache-restore selection | 5 | 8 unrelated resolver filesystem cases |
| Configuration selection | 54 | 1 process-CWD case |

All fourteen published blob entries were checked against the tested source
archive and their Git/SHA-256 identities. Final source is committed through the
connected GitHub app. The temporary integration scripts and write-enabled
workflow are removed; `body-cache-reuse.yml` runs committed source read-only.
Final-head execution is recorded separately on the PR, not inferred from the
staged pass. #63's old `action_required` checks are not approved or relabeled by
this integration.

| Runtime file | SHA-256 |
| --- | --- |
| `src/config/mod.rs` | `95d00388f6628f298a1e0b398410f3b0409231af54860d5ee37129093d9c96c8` |
| `src/indexing/facade.rs` | `f9b5e9692ae73c1aacb522083580a70daa6483222ebb03160c5d39cc9aa89207` |
| `src/indexing/pipeline/stages/semantic_embed.rs` | `d2cbdbcba482b49dcf6c01b55c052fae607c65e58fb9f15acf7db51e12468224` |
| `src/semantic/simple.rs` | `dd0bf908392f01b3d9d14937d87d426e3f8857547d3cde1c9bcb039093c13c50` |

## Earlier failures and newly identified follow-up

The initial merge stopped on an unreviewed `simple.rs` conflict; its explicit
resolution preserved body storage and added only the reviewed cache helpers.
Run 35775120044 then exposed zero planner hits after actual body indexing.
Run 35776505791 confirmed the relative-root repair but its first stress case
used 16,384 dimensions: valid for the optional cache, unsupported by the code
journal's 1..=4096 read contract. First publication could complete, while the
next save rejected that metadata. Those runs are not labeled passing eviction
experiments.

The final pressure fixture uses the supported 4,096 dimensions and increases
parent count to retain real byte pressure; no production limit was relaxed.
Repeated literal bytes additionally prove duplicate segment handling independent
of header/split alignment.

**Remaining cost-safety blocker:** reject unsupported code dimensions before
source inference and before first checkpoint publication, and expose the format
constraint in preflight. Preserve existing invalid stores rather than treating
them as empty. Issues are disabled in this repository; the reproduction and
acceptance criteria are retained on #64. This integration does not claim that
late-dimension validation is repaired.

## Remaining release/relevance boundary

The default policy remains `doc_comment`. No production index, model or paid
provider was used or rebuilt. The planner is still a source-build auxiliary
binary; release packaging is not qualified here.

#62's lexical/scope/related-code runtime, wider all-branch release gates, cold/warm
profiling and independent semantic-quality judgments remain separate. The frozen
source/oracle is not changed, and no new relevance gain is claimed. A complete
input plan is not proof that a full body-index rebuild is worthwhile.
