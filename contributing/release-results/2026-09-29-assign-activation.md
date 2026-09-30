# Assign activation — 2026-09-29

The user authorized replacing the installed binary and force rebuilding the Assign index after the RC4 remediation. This receipt records local activation; the original failed qualification report remains historical evidence.

## Installed state

- Binary: `/Users/bpstr/.local/bin/codanna`, repaired RC4 build, emission semantics v10.
- SHA256: `1774f72bfda3b28edab9531696e22396a481f31c495a5066e9c65035112928dc`.
- Active index: `/Users/bpstr/Github/assign/.codanna/index`.
- Code: 2,813 files, 129,001 symbols, 59,130 relationships.
- Documents: 1,265 files, 36,582 chunks and live vectors, zero unembedded chunks.
- Existing MiniLM and code `doc_comment` settings were preserved. Multilingual evaluation remains deferred; this activation does not establish multilingual quality or default code retrieval quality.

## Verification

Force code indexing completed in 39.03 seconds. Force document indexing completed in 1,310.72 seconds. Four document files changed during the rebuild; incremental catch-up completed, and all 1,265 recorded source hashes then matched. Catch-up leaves 37,101 physical document vectors for 36,582 live chunks.

Installed CLI metadata and document statistics succeeded after activation. Actual MCP `get_index_info` and `find_symbol` for `SearchableSettingsPicker` succeeded against Assign. MCP semantic generation alignment remains untracked in the existing metadata; no vector-alignment claim is made.

The initial drift command exceeded the CLI's 1,000-file cap and exited 2. The corrected bounded drift check succeeded but was truncated; full recorded-file hash comparison supplies the complete known-file freshness evidence. Original failure output is retained.

The rebuild used cached local model weights without paid inference. Original settings SHA256 remained `f1632864ed079ea8b6a1a414e75998f9f79ad61520ba319df816c10b4c490269`. Neither index had open handles before activation. Darwin atomic directory exchange activated the rebuilt index, followed by atomic executable replacement.

## Evidence and rollback

Evidence directory: `/Users/bpstr/Github/codanna-release-evidence/2026-09-29-assign-activation`.

`activation.json`, `mcp-verification.json`, installed CLI receipts, rebuild logs, and both document hash checks are retained there. The previous executable is `old-codanna`; previous index copies are `previous-active-index` and `rollback-index`. No release was published by this operation.
