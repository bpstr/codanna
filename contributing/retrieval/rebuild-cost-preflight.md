# Offline code rebuild preflight

Implementation: [PR #49](https://github.com/bpstr/codanna/pull/49), stacked on
[compatible rebuild reuse, PR #47](https://github.com/bpstr/codanna/pull/47).
This is a source-input inventory, **not a quote for provider tokens or money**.
It does not decide whether retrieval quality justifies a rebuild.

## Run the source-built command

Build the auxiliary binary from the reviewed branch in the Codanna checkout:

```bash
cargo build --locked --bin codanna-index-plan
```

Then, from the workspace being inspected, use that binary's absolute path:

```bash
/path/to/codanna/target/debug/codanna-index-plan \
  --config .codanna/settings.toml
```

Optional positional paths select source roots instead of configured indexed
paths. Relative arguments resolve against the process working directory.
All roots must be within the configured workspace. An existing settings file is
required: the command does not discover, register, initialize, or repair a
workspace. The auxiliary binary avoids the ordinary indexing startup path;
this is not a new `codanna index --dry-run` guarantee. Release packages have not
yet been qualified to ship this additional binary. Building the command is a
separate Cargo operation that can download build dependencies; the **running
preflight** does not initialize an embedding backend or download models/grammars.

Keep stdout in a report outside the source roots when saving it, and inspect the
exit status as well as JSON. The command itself creates no report file.

## What the JSON means

| Field | Interpretation |
| --- | --- |
| `status` | Source inventory is `complete`, `partial`, `blocked`, or `partial_blocked`; complete does not mean exact cost is known. |
| `source_fingerprint` | Hash of sorted source paths and content hashes. Identifies the observed files, not an atomic filesystem/index generation. |
| `files_discovered`, `files_parsed`, `files_requiring_generic_parser` | Supported source discovery, native files actually parsed, and generic files deliberately left unparsed. |
| `symbols`, `symbols_without_embedding_input` | Parser output and symbols without the currently supported comment input. These are not semantic recall metrics. |
| `embedding_candidates`, `unique_embedding_inputs`, `duplicate_embedding_inputs` | Comment-input occurrences versus distinct exact input strings. Duplicate candidates can still incur repeated work in runtime batches. |
| `embedding_input_bytes`, `unique_embedding_input_bytes` | UTF-8 input sizes. Never reinterpret these as billed token counts. |
| `snapshot_hit_inputs`, `snapshot_miss_inputs`, `snapshot_unique_miss_inputs` | Matches against the bounded, read-only cache snapshot under the configured remote identity/dimension. Matches are potential reuse, not guaranteed future hits. |
| `cache_present` | Whether a regular cache file exists. It does not certify the file is valid or compatible. Invalid/incompatible cache entries are misses under the existing runtime loader. |
| `configured_identity_sha256` | A fingerprint of configuration-derived identity, not proof of the live provider's deployment identity. |
| `cache_lookup` | Explains a checked remote snapshot, disabled semantic indexing, unknown local identity, or missing remote dimension. Unknown counts are null, not zero. |
| `input_policy_rejections` | Parsed inputs rejected by budget or tokenizer validation. A blocked report is not a predicted successful rebuild. |
| `exact_provider_tokens` | Null: this implementation makes no exact billing-token or price estimate. |
| `provider_requests_made` | Zero for the preflight. A later actual remote rebuild may still make an initialization probe. |

Exit **0** means complete source inventory with no observed input rejection.
Exit **3** accompanies a partial or blocked JSON report. Exit **1** is a fatal
configuration, filesystem, parser, or safety-limit failure with no success
report. Argument parsing may exit **2**. Unknown cache/model identity can coexist
with complete source inventory; inspect nullable fields rather than treating
exit 0 as permission for a zero-cost rebuild.

## Read-only and bounded behavior

The planner uses the existing bounded file walker, native parse stage, actual
comment-input eligibility, input-budget validation, and compatible cache loader.
It does not construct an IndexFacade/DocumentIndex, read active vector mappings,
contact the embedding endpoint, or discover/import conversation history.
Existing corrupt index documents are irrelevant to this source-only inventory.

Generic language-pack files are skipped even when a grammar might already be
cached, because that path can initialize/download grammar resources. Their
unknown input population makes the report partial. Lua has a native registered
parser and is parsed; Zig is used as the generic negative control. Local model
identity remains unknown instead of loading a tokenizer from a model. A remote
tokenizer is read only when explicitly configured and must be a regular file;
otherwise the existing byte-budget proxy is used for admission checks, not
billed-token estimation. Remote dimensions are not probed. The normal remote
URL/model/dimension environment overrides apply.

Discovery is bounded at 200,000 entries, 25,000 source files, and 128 MiB of source
contents; at most 100,000 distinct input hashes are retained. Cache file/vector
budgets remain those of the runtime loader. Exceeding a bound is an error, not a
silently truncated successful inventory. Overlapping roots are deduplicated;
ignore rules still apply. Successful reports omit source text, endpoint URLs,
API keys and individual input hashes, but do include selected root paths.
Read-only means no application-created/modified workspace files or cache writes;
ordinary filesystem access-time updates by the operating system are not tested.

## Executed cache-pressure experiment

[Run 35671574708](https://github.com/bpstr/codanna/actions/runs/35671574708)
compiled and executed three cache-policy tests at source checkout
`167a36fe3ca54faa00f055d779099148e16d7c13` (PR head `be659744`).
All three passed. The subsequent CLI test compilation failed on its SHA-256
formatting helper, before CLI cases ran; it is not counted as CLI verification.

The experiment uses the production cache with fixed vectors and explicit
64-item lookup-then-admission batches. It does not contact a provider or measure
a real workspace's token use or indexing order.

| Synthetic case | Observed result |
| --- | --- |
| 4,500 distinct inputs, two-dimensional vectors, initial resident tail | 4,096 input matches in the initial cache snapshot. |
| Scan in original order and admit misses after each 64-item batch | **0 reused inputs**: earlier misses evicted entries before their turn. |
| Same scan without admitting misses (experimental control only) | 4,096 matches; this does not learn new inputs for future runs. |
| Reverse-order scan with normal admission | 4,096 matches; no runtime ordering change is selected here. |
| 1,400 inputs with 3,072-dimensional fixed vectors | 1,337 resident entries: the 16 MiB logical byte budget binds before the 4,096-entry ceiling. |
| Save/reload then repeat original scan | Same pressure boundary; the persisted cache bytes remain unchanged by the simulation. |

The byte-budget case includes the existing 256-byte per-entry accounting. It is
not a measurement of process RSS. These results explain why a preflight must not
promise that initial cache hits equal final embedding savings. The streaming
embedding stage looks up and admits bounded batches, so ordering is a meaningful
risk; this simulation is not an exhaustive reproduction of all scheduling paths.
No cache capacity, admission policy, model, or embedding input policy is changed
by this PR. A policy repair needs bounded-memory and full rebuild/next-rebuild
controls, not just the no-admission experiment's favorable single-pass number.

## Verification and remaining scope

The subprocess fixtures use disposable HOME/workspace directories, an empty
child environment, synthetic credential text only, and a local listening socket
that fails the test on any connection. They compare file contents, directory
membership and modification timestamps before/after each invocation. Fixtures
cover source edits, compatible/incompatible/corrupt caches, overlaps, ignores,
duplicates, absent/corrupt indexes, unknown/disabled backends, input rejection,
invalid roots/configuration, non-regular cache/tokenizer paths, and generic-parser
partial coverage alongside native Lua. Tokenizer encoding errors produce a
blocked report, not fabricated token counts.

The read-only focused workflow runs the three pressure tests, the CLI tests, and
existing semantic rebuild CLI regressions, followed by formatting and strict
Clippy. At `4120494`, nine CLI tests passed and the tenth exposed an incorrect
fixture assumption that Lua was generic; it was corrected to retain Lua as a
positive control and add Zig. That failed checkpoint is not a complete CLI pass.
See the PR's revision-specific verification record for later completed results;
no real index or paid endpoint is part of these tests.

The preflight covers code comment inputs, not document collections, graph
completeness, ranking quality, deployment, or a combined release of all calendar
fixes. Do not delete the existing cache to prepare a rebuild. First verify the
combined runtime, retain a recoverable backup, inspect source/input/cache
coverage, then use a separately approved hard request/monetary cap and stop
condition for any real remote reindex.
