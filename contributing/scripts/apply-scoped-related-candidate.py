#!/usr/bin/env python3
"""Apply reviewed scoped-endpoint wiring to the verified combined source."""
from pathlib import Path
import subprocess

EXPECTED = {
    "src/storage/tantivy/graph.rs": "97c36b18e13d803f40291d989f39f8ba7a62f4ea",
    "src/mcp/tools/ticket_related.rs": "2374167ecb1e9e870f85c2ec92f57b92fd229e48",
    "src/mcp/tools/ticket_related_tests.rs": "1fbb4b79e7a7e5b605328ad7f4c39d6b1f00b079",
    "tests/ticket_related_cli.rs": "72ff67140ec829df9f498ee7c3c6c8f17a087065",
}


def replace(text, old, new):
    assert text.count(old) == 1, (old, text.count(old))
    return text.replace(old, new)


def main():
    texts = {}
    for name, expected in EXPECTED.items():
        actual = subprocess.check_output(["git", "hash-object", name], text=True).strip()
        assert actual == expected, (name, actual, expected)
        texts[name] = Path(name).read_text()
    name = "src/storage/tantivy/graph.rs"
    texts[name] = replace(texts[name], "pub type GraphEdge =", '#[path = "graph_scope.rs"]\nmod endpoint_scope;\n\npub type GraphEdge =')

    name = "src/mcp/tools/ticket_related.rs"
    text = texts[name]
    text = replace(text, "    pub unhydrated_edges: usize,", "    pub unhydrated_edges: usize,\n    pub excluded_by_scope: usize,")
    text = replace(text, "pub(super) struct RelatedCode {", 'pub(super) struct RelatedCode {\n    #[serde(skip_serializing_if = "Option::is_none")]\n    pub path_prefix: Option<String>,')
    text = replace(text, "        Self {\n            status,", "        Self {\n            path_prefix: None,\n            status,")
    text = replace(text, "        for (rank, item) in self.items.iter().enumerate() {", '''        if let Some(prefix) = &self.path_prefix {
            output.push_str(&format!("Scope: {prefix}; targets require registered workspace file identity.\\n"));
        }
        for (rank, item) in self.items.iter().enumerate() {''')
    text = replace(text, "        for probe in &self.probes {", '''        for probe in &self.probes {
            if probe.excluded_by_scope > 0 {
                output.push_str(&format!("Seed {}: {} indexed edge(s) excluded by the requested file scope.\\n", probe.seed_symbol_id, probe.excluded_by_scope));
            }''')
    text = replace(text, '''    if scope.is_some() {
        return RelatedCode::unavailable("not_run_scoped_graph_unsupported");
    }
''', "")
    text = replace(text, "    let before = storage.generation();", "    let view = storage.graph_view();\n    let before = view.reader_generation();")
    text = replace(text, "    result.reader_generation_before = Some(before);", "    result.path_prefix = scope.map(|prefix| text(prefix, 512));\n    result.reader_generation_before = Some(before);")
    text = replace(text, "    // Both edge enumeration and all endpoint hydration use this pinned view.\n    let view = storage.graph_view();", "    // Scope resolution, edge enumeration and endpoint hydration share the pinned view.")
    text = replace(text, "            unhydrated_edges: 0,", "            unhydrated_edges: 0,\n            excluded_by_scope: 0,")
    text = replace(text, "            let mut edges =\n", '''            if let Some(prefix) = scope {
                if view.symbols_scoped(&[id], prefix)?.is_empty() {
                    probe.status = "seed_outside_scope";
                    return Ok(());
                }
            }
            let mut edges =
''')
    text = replace(text, "            for (_, target_id, relationship) in edges {", '''            let allowed = scope.map(|prefix| {
                view.symbols_scoped(&target_ids, prefix).map(|symbols| {
                    symbols.into_iter().map(|symbol| symbol.id.value()).collect::<BTreeSet<_>>()
                })
            }).transpose()?;
            for (_, target_id, relationship) in edges {''')
    text = replace(text, "                if excluded.contains(&target_id.value()) {", '''                if allowed.as_ref().is_some_and(|ids| !ids.contains(&target_id.value())) {
                    probe.excluded_by_scope += 1;
                    continue;
                }
                if excluded.contains(&target_id.value()) {''')
    text = replace(text, '''    } else if result.items.is_empty() {
        "empty_indexed_neighborhoods"
''', '''    } else if result.items.is_empty() {
        if scope.is_some() { "empty_scoped_neighborhoods" } else { "empty_indexed_neighborhoods" }
''')
    texts[name] = text

    name = "src/mcp/tools/ticket_related_tests.rs"
    text = texts[name]
    text = replace(text, "fn ticket_related_rejects_scope_and_generation_mismatch_before_expansion()", "fn ticket_related_rejects_unregistered_seeds_and_generation_mismatch_before_expansion()")
    text = replace(text, '''    assert_eq!(scoped.status, "not_run_scoped_graph_unsupported");
    assert!(scoped.probes.is_empty());''', '''    assert_eq!(scoped.status, "partial");
    assert_eq!(scoped.probes[0].status, "seed_outside_scope");
    assert!(scoped.probes[0].indexed_edges.is_none());
    assert!(scoped.items.is_empty());''')
    texts[name] = text

    name = "tests/ticket_related_cli.rs"
    text = texts[name]
    text = replace(text, '"include_related_code":true, "code_path_prefix":"src",', '"include_related_code":true, "code_path_prefix":"src", "code_limit":1,')
    text = replace(text, '''    assert_eq!(report["status"], "not_run_scoped_graph_unsupported");
    assert_eq!(report["probes"], json!([]));
    assert_eq!(report["items"], json!([]));''', '''    assert_eq!(report["status"], "completed_bounded");
    assert_eq!(report["items"].as_array().unwrap().len(), 1);
    assert_eq!(report["items"][0]["name"], "timeout_worker");
    assert_eq!(report["path_prefix"], "src");
    let text_result = workspace.ticket(&json!({
        "query":"conversation timeout", "include_related_code":true,
        "code_path_prefix":"src/main.rs", "code_limit":1,
    }), false);
    assert!(text_result.status.success());
    let rendered = String::from_utf8_lossy(&text_result.stdout);
    assert!(rendered.contains("Scope: src/main.rs"));
    assert!(rendered.contains("timeout_worker"));''')
    texts[name] = text
    for name, text in texts.items():
        Path(name).write_text(text)
    subprocess.run(["cargo", "fmt", "--all"], check=True)
    subprocess.run(["git", "diff", "--check"], check=True)


if __name__ == "__main__":
    main()
