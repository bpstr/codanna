//! Bounded cross-symbol inference with one source-partitioning pass.
//!
//! Retain compact source-range descriptors, not collector-sized prepared strings.
//! Pin all existing body cache hits before any comment/body cache admission, then
//! materialize at most one bounded window. The source policy and ranges are unchanged.

use crate::SymbolId;
use crate::indexing::pipeline::types::{EmbeddingBatch, PipelineError, PipelineResult};
use crate::memory::{MemoryBudget, MemorySampler};
use crate::semantic::{EmbeddingBackend, SimpleSemanticSearch, SymbolSegment};
use crate::symbol_representation::{SymbolInput, SymbolSource};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex};

const MAX_WINDOW_INPUTS: usize = 64;
// Each validated input is at most 1 MiB and a symbol selects at most 8 inputs.
// This bound admits every valid single symbol without an oversized-item bypass.
const MAX_WINDOW_BYTES: usize = 8 * 1024 * 1024;

type SharedVector = Arc<[f32]>;
type VectorSlots = Vec<Vec<Option<SharedVector>>>;

fn failure(reason: impl Into<String>) -> PipelineError {
    PipelineError::Parse {
        path: Default::default(),
        reason: reason.into(),
    }
}

fn sample_memory(sampler: &mut MemorySampler) -> PipelineResult<MemoryBudget> {
    let memory = sampler.sample();
    if memory.under_pressure() {
        return Err(failure("body embedding stopped before memory pressure"));
    }
    Ok(memory)
}

struct PreparedInput {
    fragment: Option<usize>,
    source_range: Option<Range<usize>>,
    bytes: usize,
    hash: String,
}

impl PreparedInput {
    fn capture(source: &SymbolSource, input: &SymbolInput) -> PipelineResult<Self> {
        let fragment = match &input.source_range {
            Some(range) => {
                let body = input
                    .text
                    .strip_prefix(&source.header)
                    .ok_or_else(|| failure("prepared body input lost its header"))?;
                let index = source
                    .fragments
                    .iter()
                    .position(|fragment| {
                        range.start >= fragment.range.start
                            && range.end <= fragment.range.end
                            && range.start <= range.end
                            && fragment.text.get(
                                range.start - fragment.range.start
                                    ..range.end - fragment.range.start,
                            ) == Some(body)
                    })
                    .ok_or_else(|| failure("prepared body input lost its source fragment"))?;
                Some(index)
            }
            None => {
                if input.text != source.header {
                    return Err(failure("prepared header-only input changed"));
                }
                None
            }
        };
        Ok(Self {
            fragment,
            source_range: input.source_range.clone(),
            bytes: input.text.len(),
            hash: crate::calculate_hash(&input.text),
        })
    }

    /// Reassemble exact bytes from immutable captured source, without repeating
    /// tokenizer-driven partitioning or v2 head/middle/tail selection.
    fn materialize(&self, source: &SymbolSource) -> PipelineResult<SymbolInput> {
        let mut text = String::with_capacity(self.bytes);
        text.push_str(&source.header);
        match (self.fragment, &self.source_range) {
            (Some(index), Some(range)) => {
                let fragment = source
                    .fragments
                    .get(index)
                    .ok_or_else(|| failure("prepared source fragment disappeared"))?;
                let start = range
                    .start
                    .checked_sub(fragment.range.start)
                    .ok_or_else(|| failure("prepared source range is invalid"))?;
                let end = range
                    .end
                    .checked_sub(fragment.range.start)
                    .ok_or_else(|| failure("prepared source range is invalid"))?;
                let body = fragment
                    .text
                    .get(start..end)
                    .ok_or_else(|| failure("prepared source range is not a UTF-8 slice"))?;
                text.push_str(body);
            }
            (None, None) => {}
            _ => return Err(failure("inconsistent prepared source descriptor")),
        }
        if text.len() != self.bytes {
            return Err(failure("prepared source byte count changed"));
        }
        Ok(SymbolInput {
            text,
            source_range: self.source_range.clone(),
        })
    }
}

struct PreparedSymbol<'a> {
    parent: SymbolId,
    source: &'a SymbolSource,
    language: &'a str,
    inputs: Vec<PreparedInput>,
}

pub(super) struct BodyPlan<'a> {
    symbols: Vec<PreparedSymbol<'a>>,
    pinned_hits: HashMap<String, SharedVector>,
}

impl<'a> BodyPlan<'a> {
    pub(super) fn prepare(
        batch: &'a EmbeddingBatch,
        pool: &EmbeddingBackend,
        semantic: &Mutex<SimpleSemanticSearch>,
    ) -> PipelineResult<Self> {
        let mut plan = Self {
            symbols: Vec::with_capacity(batch.body_candidates.len()),
            pinned_hits: HashMap::new(),
        };
        let mut sampler = MemorySampler::new();
        for (index, (parent, source, language, _)) in batch.body_candidates.iter().enumerate() {
            // No per-candidate reconstruction of sysinfo::System. Resample at a
            // bounded interval during preparation and before every inference call.
            if index % 32 == 0 {
                sample_memory(&mut sampler)?;
            }
            let inputs = source.inputs(pool.input_budget()).map_err(failure)?;
            let mut descriptors = Vec::with_capacity(inputs.len());
            let mut semantic = semantic
                .lock()
                .map_err(|_| failure("Failed to lock semantic search"))?;
            for input in inputs {
                let prepared = PreparedInput::capture(source, &input)?;
                if let Some(vector) = semantic.cached_symbol_input(&input.text) {
                    plan.pinned_hits
                        .entry(prepared.hash.clone())
                        .or_insert(vector);
                }
                descriptors.push(prepared);
            }
            plan.symbols.push(PreparedSymbol {
                parent: *parent,
                source,
                language,
                inputs: descriptors,
            });
        }
        Ok(plan)
    }

    pub(super) fn process(
        &self,
        pool: &EmbeddingBackend,
        semantic: &Mutex<SimpleSemanticSearch>,
    ) -> PipelineResult<usize> {
        let mut offset = 0;
        let mut sampler = MemorySampler::new();
        while offset < self.symbols.len() {
            sample_memory(&mut sampler)?;
            let end = window_end(&self.symbols, offset);
            if end == offset {
                return Err(failure(
                    "body symbol exceeds the validated input window bound",
                ));
            }
            self.process_window(&self.symbols[offset..end], pool, semantic, &mut sampler)?;
            offset = end;
        }
        Ok(self.symbols.len())
    }

    fn process_window(
        &self,
        symbols: &[PreparedSymbol<'_>],
        pool: &EmbeddingBackend,
        semantic: &Mutex<SimpleSemanticSearch>,
        sampler: &mut MemorySampler,
    ) -> PipelineResult<()> {
        let inputs: Vec<Vec<SymbolInput>> = symbols
            .iter()
            .map(|symbol| {
                symbol
                    .inputs
                    .iter()
                    .map(|input| input.materialize(symbol.source))
                    .collect()
            })
            .collect::<PipelineResult<_>>()?;
        let languages: Vec<_> = symbols.iter().map(|symbol| symbol.language).collect();
        let mut vectors: VectorSlots = inputs.iter().map(|row| vec![None; row.len()]).collect();
        {
            let mut semantic = semantic
                .lock()
                .map_err(|_| failure("Failed to lock semantic search"))?;
            for (parent, row) in inputs.iter().enumerate() {
                for (segment, input) in row.iter().enumerate() {
                    vectors[parent][segment] = self
                        .pinned_hits
                        .get(&symbols[parent].inputs[segment].hash)
                        .cloned()
                        .or_else(|| semantic.cached_symbol_input(&input.text));
                }
            }
        }
        let groups = group_missing(&inputs, &languages, &vectors);
        let mut offset = 0;
        while offset < groups.len() {
            let memory = sample_memory(sampler)?;
            let count = memory.embedding_batch_size(
                MAX_WINDOW_INPUTS,
                crate::memory::accelerated_embeddings_requested(),
            );
            let end = (offset + count).min(groups.len());
            // Request-local ordinals identify unique inputs, never persisted IDs.
            let request: Vec<_> = (offset..end)
                .map(|index| {
                    (
                        SymbolId::new(index as u32 + 1).expect("bounded input ordinal"),
                        groups[index].text,
                        groups[index].language,
                    )
                })
                .collect();
            let results = pool
                .embed_parallel(&request)
                .map_err(|error| failure(error.to_string()))?;
            scatter_results(
                &groups,
                offset..end,
                results,
                &mut vectors,
                pool.dimensions(),
            )?;
            offset = end;
        }
        // No model work under the semantic lock. Publish only complete parents,
        // after the entire bounded window has been checked for missing results.
        let mut semantic = semantic
            .lock()
            .map_err(|_| failure("Failed to lock semantic search"))?;
        for (index, symbol) in symbols.iter().enumerate() {
            let segments = std::mem::take(&mut vectors[index])
                .into_iter()
                .zip(&inputs[index])
                .map(|(vector, input)| {
                    vector
                        .map(|vector| SymbolSegment::new(input.source_range.clone(), vector))
                        .ok_or_else(|| failure("missing symbol segment embedding"))
                })
                .collect::<PipelineResult<Vec<_>>>()?;
            semantic
                .store_symbol_segments(symbol.parent, segments, &inputs[index], symbol.language)
                .map_err(|error| failure(error.to_string()))?;
        }
        Ok(())
    }
}

fn window_end(symbols: &[PreparedSymbol<'_>], start: usize) -> usize {
    let mut end = start;
    let mut count = 0usize;
    let mut bytes = 0usize;
    for symbol in &symbols[start..] {
        let next_count = count.saturating_add(symbol.inputs.len());
        let next_bytes = symbol
            .inputs
            .iter()
            .fold(bytes, |sum, input| sum.saturating_add(input.bytes));
        if next_count > MAX_WINDOW_INPUTS || next_bytes > MAX_WINDOW_BYTES {
            break;
        }
        count = next_count;
        bytes = next_bytes;
        end += 1;
    }
    end
}

struct MissingGroup<'a> {
    text: &'a str,
    language: &'a str,
    targets: Vec<(usize, usize)>,
}

fn group_missing<'a>(
    inputs: &'a [Vec<SymbolInput>],
    languages: &'a [&str],
    vectors: &VectorSlots,
) -> Vec<MissingGroup<'a>> {
    let mut indexes: HashMap<&str, usize> = HashMap::new();
    let mut groups: Vec<MissingGroup<'a>> = Vec::new();
    for (parent, row) in inputs.iter().enumerate() {
        for (segment, input) in row.iter().enumerate() {
            if vectors[parent][segment].is_some() {
                continue;
            }
            if let Some(&index) = indexes.get(input.text.as_str()) {
                groups[index].targets.push((parent, segment));
            } else {
                indexes.insert(&input.text, groups.len());
                groups.push(MissingGroup {
                    text: &input.text,
                    language: languages[parent],
                    targets: vec![(parent, segment)],
                });
            }
        }
    }
    groups
}

fn scatter_results(
    groups: &[MissingGroup<'_>],
    requested: Range<usize>,
    results: Vec<(SymbolId, Vec<f32>, String)>,
    vectors: &mut VectorSlots,
    dimensions: usize,
) -> PipelineResult<()> {
    if results.len() != requested.len() {
        return Err(failure("incomplete body embedding batch"));
    }
    let mut seen = vec![false; groups.len()];
    // Validate the complete response before installing even an in-memory result.
    for (id, vector, _) in &results {
        let index = id.value() as usize - 1;
        if !requested.contains(&index) || index >= groups.len() || seen[index] {
            return Err(failure("unexpected or duplicate body embedding ordinal"));
        }
        if vector.len() != dimensions || !vector.iter().all(|value| value.is_finite()) {
            return Err(failure("invalid body embedding vector"));
        }
        seen[index] = true;
    }
    for (id, vector, _) in results {
        let shared: SharedVector = Arc::from(vector);
        for &(parent, segment) in &groups[id.value() as usize - 1].targets {
            let slot = vectors
                .get_mut(parent)
                .and_then(|row| row.get_mut(segment))
                .ok_or_else(|| failure("invalid body embedding target"))?;
            if slot.is_some() {
                return Err(failure("duplicate body embedding target"));
            }
            *slot = Some(Arc::clone(&shared));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding_input::InputBudget;
    use crate::symbol_representation::{CodeEmbeddingPolicy, SourceFragment};

    fn input(text: &str) -> SymbolInput {
        SymbolInput {
            text: text.into(),
            source_range: None,
        }
    }

    fn source() -> SymbolSource {
        let text = "first közép last\n".repeat(200);
        let len = text.len();
        SymbolSource {
            header: "function f\n".into(),
            fragments: vec![SourceFragment {
                range: 42..42 + len,
                text,
            }],
            policy: CodeEmbeddingPolicy::SymbolBodyV2,
        }
    }

    #[test]
    fn prepared_ranges_recreate_exact_v2_inputs_without_repartitioning() {
        let source = source();
        let budget = InputBudget::remote(Some(100), None).unwrap();
        let original = source.inputs(&budget).unwrap();
        assert_eq!(
            original.len(),
            crate::symbol_representation::MAX_SYMBOL_SEGMENTS
        );
        for input in &original {
            let prepared = PreparedInput::capture(&source, input).unwrap();
            let restored = prepared.materialize(&source).unwrap();
            assert_eq!(restored.text, input.text);
            assert_eq!(restored.source_range, input.source_range);
            assert_eq!(prepared.hash, crate::calculate_hash(&restored.text));
        }
    }

    #[test]
    fn header_only_inputs_preserve_bytes_and_missing_range() {
        let mut source = source();
        source.fragments.clear();
        let original = input(&source.header);
        let restored = PreparedInput::capture(&source, &original)
            .unwrap()
            .materialize(&source)
            .unwrap();
        assert_eq!(restored.text, source.header);
        assert_eq!(restored.source_range, None);
    }

    #[test]
    fn cross_symbol_duplicates_are_inferred_once_and_scattered_by_ordinal() {
        let inputs = vec![vec![input("same"), input("other")], vec![input("same")]];
        let mut vectors = vec![vec![None, None], vec![None]];
        let languages = ["rust", "go"];
        let groups = group_missing(&inputs, &languages, &vectors);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].targets, vec![(0, 0), (1, 0)]);
        // Backends may complete in a different order from request order.
        let results = vec![
            (SymbolId::new(2).unwrap(), vec![0.0, 1.0], "rust".into()),
            (SymbolId::new(1).unwrap(), vec![1.0, 0.0], "rust".into()),
        ];
        scatter_results(&groups, 0..2, results, &mut vectors, 2).unwrap();
        assert_eq!(vectors[0][1].as_deref(), Some([0.0, 1.0].as_slice()));
        assert!(Arc::ptr_eq(
            vectors[0][0].as_ref().unwrap(),
            vectors[1][0].as_ref().unwrap()
        ));
    }

    #[test]
    fn entirely_cached_window_has_no_inference_groups() {
        let inputs = vec![vec![input("cached")]];
        let vectors = vec![vec![Some(Arc::<[f32]>::from(vec![1.0, 0.0]))]];
        assert!(group_missing(&inputs, &["rust"], &vectors).is_empty());
    }

    #[test]
    fn malformed_responses_do_not_install_partial_results() {
        let inputs = vec![vec![input("a"), input("b")]];
        let languages = ["rust"];
        let cases = vec![
            vec![],
            vec![(1, vec![1.0, 0.0]), (1, vec![0.0, 1.0])],
            vec![(1, vec![1.0, 0.0]), (3, vec![0.0, 1.0])],
            vec![(1, vec![1.0, 0.0]), (2, vec![f32::NAN, 1.0])],
            vec![(1, vec![1.0, 0.0]), (2, vec![1.0])],
        ];
        for case in cases {
            let mut vectors = vec![vec![None, None]];
            let groups = group_missing(&inputs, &languages, &vectors);
            let results = case
                .into_iter()
                .map(|(id, vector)| (SymbolId::new(id).unwrap(), vector, "rust".into()))
                .collect();
            assert!(scatter_results(&groups, 0..2, results, &mut vectors, 2).is_err());
            assert!(vectors[0].iter().all(Option::is_none));
        }
    }

    #[test]
    fn windows_bound_both_segment_count_and_input_bytes() {
        let source = source();
        let symbols: Vec<_> = (1..=65)
            .map(|id| PreparedSymbol {
                parent: SymbolId::new(id).unwrap(),
                source: &source,
                language: "rust",
                inputs: vec![PreparedInput {
                    fragment: None,
                    source_range: None,
                    bytes: 1,
                    hash: String::new(),
                }],
            })
            .collect();
        assert_eq!(window_end(&symbols, 0), 64);
        assert_eq!(window_end(&symbols, 64), 65);
        let mut large = symbols;
        for symbol in &mut large {
            symbol.inputs[0].bytes = MAX_WINDOW_BYTES / 2;
        }
        assert_eq!(window_end(&large, 0), 2);
    }
}
