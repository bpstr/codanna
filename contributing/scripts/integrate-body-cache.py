#!/usr/bin/env python3
"""Temporary local integration recipe. Never updates a remote ref or runs inference."""
import pathlib
import subprocess
import sys

BODY = "1662e95e14323dbd197a381ff9789d103df8a538"
CACHE = "00f8c7daba1fc1ac683469c7c6a8c214fddd01f0"
STAGE = "src/indexing/pipeline/stages/semantic_embed.rs"
FACADE = "src/indexing/facade.rs"
SIMPLE = "src/semantic/simple.rs"


def git(*args):
    return subprocess.check_output(["git", *args], text=True)


def replace_once(text, old, new):
    if text.count(old) != 1:
        raise SystemExit(f"Expected exactly one reviewed block: {old[:100]!r}")
    return text.replace(old, new, 1)


def combine():
    git("merge-base", "--is-ancestor", BODY, "HEAD")
    subprocess.run(["git", "merge", "--no-commit", "--no-ff", CACHE], check=False)
    conflicts = set(git("diff", "--name-only", "--diff-filter=U").splitlines())
    if not conflicts.issubset({STAGE, FACADE, SIMPLE}):
        raise SystemExit(f"Unreviewed merge conflict: {conflicts}")
    if SIMPLE in conflicts:
        # Both branches insert methods at the same location. Retain body storage
        # in full and add only the three reviewed cache-reuse changes from #50.
        simple = git("show", f"{BODY}:{SIMPLE}")
        cache_simple = git("show", f"{CACHE}:{SIMPLE}")
        marker = "use crate::SymbolId;"
        simple = replace_once(simple, marker, '#[path = "rebuild_cache.rs"]\nmod rebuild_cache;\n\n' + marker)
        begin = cache_simple.index("    /// Store one generated vector for every symbol with the same exact input.")
        end = cache_simple.index("    fn insert_embedding(", begin)
        marker = "    pub(crate) fn cached_symbol_input("
        simple = replace_once(simple, marker, cache_simple[begin:end] + marker)
        begin = cache_simple.index("        // An empty vector generation can still hold compatible cached inputs")
        end = cache_simple.index("        if let Some(metadata) = &mut self.metadata {", begin)
        method_start = simple.index("    pub(crate) fn set_embedding_identity(")
        insertion = simple.index("        if let Some(metadata) = &mut self.metadata {", method_start)
        simple = simple[:insertion] + cache_simple[begin:end] + simple[insertion:]
        pathlib.Path(SIMPLE).write_text(simple)
    if FACADE in conflicts:
        facade = git("show", f"{BODY}:{FACADE}")
        marker = "        self.semantic_search = Some(Arc::new(Mutex::new(semantic)));"
        if facade.count(marker) != 1:
            raise SystemExit("Unexpected facade setup count")
        facade = facade.replace(marker, "        // Fresh IDs reuse exact inputs, never old symbol mappings.\n        semantic.restore_rebuild_cache(&semantic_path);\n\n" + marker)
        pathlib.Path(FACADE).write_text(facade)
    if "semantic.restore_rebuild_cache(&semantic_path);" not in pathlib.Path(FACADE).read_text():
        raise SystemExit("Merge lost force-rebuild cache restoration")
    body = git("show", f"{BODY}:{STAGE}")
    cache = git("show", f"{CACHE}:{STAGE}")
    begin = cache.index("    /// Process a batch of embedding candidates.")
    end = cache.index("\n}\n\n#[cfg(test)]", begin)
    method = cache[begin:end].replace("    fn process_batch(", "    pub(crate) fn process_batch(", 1)
    method = replace_once(method, "        Ok(stored)\n    }", "        for (id, source, language) in &batch.body_candidates {\n            self.process_symbol_source(*id, source, language)?;\n            stored += 1;\n        }\n        Ok(stored)\n    }")
    begin = body.index("    /// Process a batch of embedding candidates.")
    end = body.index("\n    fn process_symbol_source(", begin)
    pathlib.Path(STAGE).write_text(body[:begin] + method + "\n" + body[end:])
    tests = pathlib.Path("tests/semantic_rebuild_reuse.rs")
    tests.write_text(tests.read_text() + '\n#[path = "support/body_rebuild_cache_cases.rs"]\nmod body_rebuild_cache;\n')
    git("add", FACADE, STAGE, SIMPLE, str(tests))
    if git("diff", "--name-only", "--diff-filter=U").strip():
        raise SystemExit("Unresolved merge state")
    print("Combined only pinned body planner and cache runtime; no branch push")


def repair():
    path = pathlib.Path(STAGE)
    text = path.read_text()
    text = replace_once(text,
        "        let accelerated = crate::memory::accelerated_embeddings_requested();\n",
        "        let accelerated = crate::memory::accelerated_embeddings_requested();\n        // Pin only existing body-input hits before any admission can evict them.\n        let body_hits = self.body_cache_hits(batch)?;\n")
    text = replace_once(text,
        "            self.process_symbol_source(*id, source, language)?;",
        "            self.process_symbol_source(*id, source, language, &body_hits)?;")
    marker = "    fn process_symbol_source(\n"
    helper = '''    /// Retain Arc references to matching cache entries, not all prepared source
    /// strings. At most the bounded cache's resident vectors are pinned until
    /// this collector batch ends; new vectors still enter the normal live cache.
    fn body_cache_hits(
        &self,
        batch: &EmbeddingBatch,
    ) -> PipelineResult<std::collections::HashMap<String, Arc<[f32]>>> {
        let mut hits = std::collections::HashMap::new();
        for (_, source, _) in &batch.body_candidates {
            if crate::memory::MemoryBudget::current().under_pressure() {
                return Err(PipelineError::Parse {
                    path: Default::default(),
                    reason: "body cache lookup stopped before memory pressure".into(),
                });
            }
            let inputs = source.inputs(self.pool.input_budget()).map_err(|reason| {
                PipelineError::Parse { path: Default::default(), reason }
            })?;
            let mut semantic = self.semantic.lock().map_err(|_| PipelineError::Parse {
                path: Default::default(),
                reason: "Failed to lock semantic search".into(),
            })?;
            for input in inputs {
                if let Some(vector) = semantic.cached_symbol_input(&input.text) {
                    hits.entry(crate::calculate_hash(&input.text)).or_insert(vector);
                }
            }
        }
        Ok(hits)
    }

'''
    text = replace_once(text, marker, helper + marker)
    text = replace_once(text,
        "        language: &str,\n    ) -> PipelineResult<()> {",
        "        language: &str,\n        body_hits: &std::collections::HashMap<String, Arc<[f32]>>,\n    ) -> PipelineResult<()> {")
    text = replace_once(text,
        "                .map(|input| semantic.cached_symbol_input(&input.text))",
        "                .map(|input| {\n                    semantic.cached_symbol_input(&input.text).or_else(|| {\n                        body_hits.get(&crate::calculate_hash(&input.text)).cloned()\n                    })\n                })")
    text = replace_once(text,
        "            .filter(|(index, _)| vectors[*index].is_none())",
        "            .filter(|(index, input)| {\n                vectors[*index].is_none()\n                    && !inputs[..*index].iter().any(|earlier| earlier.text == input.text)\n            })")
    text = replace_once(text,
        "                vectors[index] = Some(Arc::from(vector));",
        "                let shared: Arc<[f32]> = Arc::from(vector);\n                for (slot, input) in inputs.iter().enumerate() {\n                    if input.text == inputs[index].text && vectors[slot].is_none() {\n                        vectors[slot] = Some(Arc::clone(&shared));\n                    }\n                }")
    path.write_text(text)
    print("Pinned body hits and deduplicated equal segments; parent commit remains atomic")


if __name__ == "__main__":
    {"combine": combine, "repair": repair}[sys.argv[1]]()
