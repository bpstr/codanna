//! Validate complete embedding inputs before inference, without altering evidence.
//!
//! Exact counts include normalization and model special tokens. A remote endpoint
//! without a supplied tokenizer uses an explicitly named byte-budget proxy; it is
//! deliberately conservative for common tokenizers, not a universal token bound.

use std::io::Read;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
use tokenizers::Tokenizer;

pub(crate) const INPUT_POLICY_VERSION: &str = "complete-input-v2";
const DEFAULT_REMOTE_BUDGET: usize = 8192;
const MAX_INPUT_BYTES: usize = 1024 * 1024;
const MAX_TOKENIZER_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const DOCUMENT_SPLITTING_POLICY: &str = "document-budget-splits-v1";
const MAX_DOCUMENT_SPLIT_PROBES: usize = 128;
const MAX_DOCUMENT_SPLIT_PARTS: usize = 65_536;

/// Check amplification before a document store clones headings for public hook output.
pub(crate) fn document_output_bytes(
    prefix_bytes: usize,
    body_bytes: usize,
    parts: usize,
) -> Result<usize, String> {
    if parts > MAX_DOCUMENT_SPLIT_PARTS {
        return Err("Document splitting exceeded the 65536-part safety limit for one character chunk; increase the input budget or reduce heading context.".into());
    }
    let allowance = prefix_bytes
        .saturating_add(body_bytes)
        .saturating_mul(64)
        .max(16 * 1024);
    let bytes = parts.checked_mul(prefix_bytes).and_then(|n| n.checked_add(body_bytes))
        .filter(|n| *n <= allowance).ok_or_else(||
            "Document splitting exceeded its bounded output-size allowance for repeated heading breadcrumbs; increase the input budget or shorten heading context. No input was truncated.".to_string()
        )?;
    Ok(bytes)
}

#[derive(Clone)]
pub(crate) struct InputBudget {
    tokenizer: Option<Arc<Tokenizer>>,
    limit: usize,
    identity: String,
}

impl InputBudget {
    pub(crate) fn remote(
        limit: Option<usize>,
        tokenizer_path: Option<&Path>,
    ) -> Result<Self, String> {
        let limit = limit.unwrap_or(DEFAULT_REMOTE_BUDGET);
        if let Some(path) = tokenizer_path {
            let file = std::fs::File::open(path).map_err(|error| {
                format!(
                    "Cannot open embedding tokenizer {}: {error}",
                    path.display()
                )
            })?;
            if file.metadata().map_err(|error| error.to_string())?.len() > MAX_TOKENIZER_BYTES {
                return Err("Embedding tokenizer JSON exceeds the 64 MiB file limit".into());
            }
            let mut bytes = Vec::new();
            file.take(MAX_TOKENIZER_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| format!("Cannot read embedding tokenizer: {error}"))?;
            if bytes.len() as u64 > MAX_TOKENIZER_BYTES {
                return Err("Embedding tokenizer JSON exceeds the 64 MiB file limit".into());
            }
            let tokenizer = Tokenizer::from_bytes(&bytes)
                .map_err(|error| format!("Invalid embedding tokenizer JSON: {error}"))?;
            Self::exact(tokenizer, limit)
        } else {
            Self::validate_limit(limit)?;
            Ok(Self {
                tokenizer: None,
                limit,
                identity: format!("{INPUT_POLICY_VERSION}:utf8-byte-budget-proxy:{limit}"),
            })
        }
    }

    /// Clone the actual inference tokenizer, retaining its model-specific ceiling.
    pub(crate) fn local(tokenizer: &Tokenizer, limit: Option<usize>) -> Result<Self, String> {
        if let Some(limit) = limit {
            Self::validate_limit(limit)?;
        }
        let model_limit = tokenizer
            .get_truncation()
            .map(|parameters| parameters.max_length)
            .ok_or_else(|| "Local embedding tokenizer has no declared input limit".to_string())?;
        let limit = limit.unwrap_or(model_limit).min(model_limit);
        Self::exact(tokenizer.clone(), limit)
    }

    pub(crate) fn validate_limit(limit: usize) -> Result<(), String> {
        if limit == 0 || limit > MAX_INPUT_BYTES {
            return Err(format!(
                "semantic_search.max_input_tokens must be between 1 and {MAX_INPUT_BYTES}"
            ));
        }
        Ok(())
    }

    fn exact(mut tokenizer: Tokenizer, limit: usize) -> Result<Self, String> {
        Self::validate_limit(limit)?;
        // Counting through the inference tokenizer with truncation still enabled
        // would report only the prefix. Padding must not inflate the count either.
        tokenizer
            .with_truncation(None)
            .map_err(|error| error.to_string())?;
        tokenizer.with_padding(None);
        let serialized = tokenizer
            .to_string(false)
            .map_err(|error| error.to_string())?;
        let digest = crate::indexing::file_info::calculate_hash(&serialized);
        Ok(Self {
            tokenizer: Some(Arc::new(tokenizer)),
            limit,
            identity: format!(
                "{INPUT_POLICY_VERSION}:huggingface-tokenizers-0.22.2:{digest}:{limit}"
            ),
        })
    }

    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }

    /// Partition a document body into independently valid complete inputs.
    ///
    /// Breadcrumbs remain intact, and ranges partition the original UTF-8 bytes
    /// without adding overlap. Search is bounded and conservative: token counts
    /// need not increase with prefix length, and no optimal partition is promised.
    pub(crate) fn document_ranges(
        &self,
        prefix: &str,
        body: &str,
    ) -> Result<Vec<Range<usize>>, String> {
        if body.is_empty() {
            self.validate([prefix])?;
            return Ok(Vec::new());
        }
        if self.tokenizer.is_none() {
            return self.document_byte_ranges(prefix, body);
        }
        let capacity = MAX_INPUT_BYTES.checked_sub(prefix.len()).filter(|n| *n > 0)
            .ok_or_else(|| "Document heading breadcrumbs leave no body capacity within the embedding byte safety limit; shorten the heading context.".to_string())?;
        // Bound repeated normalization/tokenization, including repeated heading
        // text. Large inputs get a linear allowance, while small inputs can still
        // explore nonmonotonic prefixes. Each probe also obeys MAX_INPUT_BYTES.
        let mut work_left = prefix
            .len()
            .saturating_add(body.len())
            .saturating_mul(64)
            .max(16 * 1024);
        let mut input = String::new();
        let mut ranges = Vec::new();
        let mut start = 0;
        let mut window = capacity;
        while start < body.len() {
            if ranges.len() == MAX_DOCUMENT_SPLIT_PARTS {
                return Err("Document splitting exceeded the 65536-part safety limit for one character chunk; increase the input budget or reduce heading context.".into());
            }
            let remaining = &body[start..];
            let mut end = remaining.len().min(window).min(capacity);
            while !remaining.is_char_boundary(end) {
                end -= 1;
            }
            if end == 0 {
                return Err("Document heading breadcrumbs leave no capacity for the next complete UTF-8 character; increase the input budget or shorten the heading context.".into());
            }
            let search_end = end;
            let mut tested = Vec::new();
            let mut fit = None;
            // Halving is only a probe order, never a monotonicity assumption.
            loop {
                tested.push(end);
                if self.document_probe(prefix, &remaining[..end], &mut input, &mut work_left)? {
                    fit = Some(end);
                    break;
                }
                if end == remaining.chars().next().expect("nonempty body").len_utf8() {
                    break;
                }
                end /= 2;
                while !remaining.is_char_boundary(end) {
                    end -= 1;
                }
                end = end.max(remaining.chars().next().expect("nonempty body").len_utf8());
            }
            // A single character may expand during normalization, yet a longer
            // prefix can merge into fewer tokens. Try other bounded candidates
            // before failing; never equate a failed singleton with impossibility.
            if fit.is_none() {
                for (offset, ch) in remaining[..search_end].char_indices() {
                    let candidate = offset + ch.len_utf8();
                    if tested.contains(&candidate) {
                        continue;
                    }
                    if tested.len() == MAX_DOCUMENT_SPLIT_PROBES {
                        break;
                    }
                    tested.push(candidate);
                    if self.document_probe(
                        prefix,
                        &remaining[..candidate],
                        &mut input,
                        &mut work_left,
                    )? {
                        fit = Some(candidate);
                        break;
                    }
                }
            }
            let end = fit.ok_or_else(|| format!(
                "Could not find a complete document split at body byte {start} within the bounded input-budget search (budget {}). The full heading breadcrumbs and model special tokens are included; increase max_input_tokens or shorten heading context. No input was truncated.", self.limit
            ))?;
            ranges.push(start..start + end);
            start += end;
            // Carry the observed scale forward instead of repeatedly tokenizing
            // a large remainder when only a small body fits beside the headings.
            window = end.saturating_mul(2).max(4).min(capacity);
        }
        Ok(ranges)
    }

    /// The byte proxy has an exact additive size, so fill its capacity directly.
    fn document_byte_ranges(&self, prefix: &str, body: &str) -> Result<Vec<Range<usize>>, String> {
        let capacity = self.limit.min(MAX_INPUT_BYTES).checked_sub(prefix.len())
            .filter(|n| *n > 0).ok_or_else(||
                "Document heading breadcrumbs leave no body budget under the UTF-8 byte-budget proxy; increase max_input_tokens or shorten heading context. No input was truncated.".to_string()
            )?;
        let mut ranges = Vec::new();
        let mut start = 0;
        let mut output_left = prefix
            .len()
            .saturating_add(body.len())
            .saturating_mul(64)
            .max(16 * 1024);
        while start < body.len() {
            if ranges.len() == MAX_DOCUMENT_SPLIT_PARTS {
                return Err("Document splitting exceeded the 65536-part safety limit for one character chunk; increase the input budget or reduce heading context.".into());
            }
            let mut end = start.saturating_add(capacity).min(body.len());
            while !body.is_char_boundary(end) {
                end -= 1;
            }
            if end == start {
                return Err("Document heading breadcrumbs leave insufficient budget for the next complete UTF-8 character under the byte-budget proxy; increase max_input_tokens or shorten heading context. No input was truncated.".into());
            }
            // Range count alone does not bound repeated heading allocations.
            output_left = output_left.checked_sub(prefix.len() + end - start)
                .ok_or_else(|| "Document splitting exceeded its bounded output-size allowance for repeated heading breadcrumbs; increase the input budget or shorten heading context. No input was truncated.".to_string())?;
            ranges.push(start..end);
            start = end;
        }
        Ok(ranges)
    }

    fn document_probe(
        &self,
        prefix: &str,
        body: &str,
        input: &mut String,
        work_left: &mut usize,
    ) -> Result<bool, String> {
        let bytes = prefix.len() + body.len();
        *work_left = work_left.checked_sub(bytes.max(1)).ok_or_else(||
            "Document splitting exhausted its bounded tokenizer-work allowance; increase the input budget or shorten heading context. No input was truncated.".to_string()
        )?;
        input.clear();
        input.push_str(prefix);
        input.push_str(body);
        Ok(self.count(input)?.0 <= self.limit)
    }

    fn count(&self, text: &str) -> Result<(usize, &'static str), String> {
        match &self.tokenizer {
            Some(tokenizer) => Ok((
                tokenizer
                    .encode(text, true)
                    .map_err(|error| error.to_string())?
                    .len(),
                "tokens (including special tokens)",
            )),
            None => Ok((
                text.len(),
                "UTF-8 bytes (token-budget proxy; exact tokenizer unavailable)",
            )),
        }
    }

    /// Preflight the entire batch before the first provider request or inference.
    pub(crate) fn validate<'a>(
        &self,
        texts: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), String> {
        for (index, text) in texts.into_iter().enumerate() {
            if text.len() > MAX_INPUT_BYTES {
                return Err(format!(
                    "Embedding input {index} is {} bytes, exceeding the {MAX_INPUT_BYTES}-byte safety limit; input was not truncated. Reduce document chunk size or heading length.",
                    text.len()
                ));
            }
            let (count, unit) = self.count(text).map_err(|error| {
                format!("Cannot tokenize complete embedding input {index}: {error}")
            })?;
            if count > self.limit {
                return Err(format!(
                    "Embedding input {index} uses {count} {unit}, exceeding the configured budget of {}; input was not truncated. The budget includes heading breadcrumbs. Reduce document chunk size or heading length, or configure the provider's tokenizer_path and max_input_tokens.",
                    self.limit
                ));
            }
        }
        Ok(())
    }
}

/// Stable, unambiguous metadata representation. No endpoint URL or credential is
/// persisted; endpoint identity is supplied as a digest by the remote backend.
pub(crate) fn backend_identity(
    backend: &str,
    model: &str,
    endpoint: Option<&str>,
    revision: Option<&str>,
    budget: &InputBudget,
) -> String {
    serde_json::json!({
        "backend": backend,
        "model": model,
        "endpoint_sha256": endpoint,
        "model_revision": revision,
        "input_policy": budget.identity(),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokenizers::models::wordlevel::WordLevel;
    use tokenizers::normalizers::unicode::NFKC;
    use tokenizers::pre_tokenizers::whitespace::Whitespace;
    use tokenizers::processors::template::TemplateProcessing;

    fn tokenizer() -> Tokenizer {
        let model = WordLevel::builder()
            .vocab([(String::from("[UNK]"), 0)].into_iter().collect())
            .unk_token("[UNK]".into())
            .build()
            .unwrap();
        let mut tokenizer = Tokenizer::new(model);
        tokenizer.with_normalizer(Some(NFKC));
        tokenizer.with_pre_tokenizer(Some(Whitespace));
        tokenizer.with_post_processor(Some(
            TemplateProcessing::builder()
                .try_single("[CLS] $A [SEP]")
                .unwrap()
                .special_tokens(vec![("[CLS]", 1), ("[SEP]", 2)])
                .build()
                .unwrap(),
        ));
        tokenizer
    }

    #[test]
    fn byte_proxy_labels_estimate_and_rejects_utf8_without_slicing() {
        let policy = InputBudget::remote(Some(12), None).unwrap();
        policy.validate(["你好世界"]).unwrap();
        let error = policy.validate(["你好世界🙂"]).unwrap_err();
        assert!(error.contains("16 UTF-8 bytes"));
        assert!(error.contains("proxy"));
        assert!(error.contains("not truncated"));
        assert!(policy.validate(["x".repeat(13).as_str()]).is_err());
    }

    #[test]
    fn exact_count_includes_normalization_expansion_and_special_tokens() {
        // U+FDFA expands under NFKC into four Arabic words. Word-level unknown
        // tokens preserve that normalized word count without a model download.
        let policy = InputBudget::exact(tokenizer(), 5).unwrap();
        policy.validate(["simple words"]).unwrap();
        let error = policy.validate(["ﷺ"]).unwrap_err();
        assert!(error.contains("6 tokens"), "{error}");
    }

    #[test]
    fn exact_count_disables_existing_truncation_and_budgets_breadcrumbs() {
        let mut tokenizer = tokenizer();
        tokenizer
            .with_truncation(Some(tokenizers::TruncationParams {
                max_length: 5,
                ..Default::default()
            }))
            .unwrap();
        let policy = InputBudget::local(&tokenizer, None).unwrap();
        policy.validate(["body words"]).unwrap();
        let error = policy
            .validate(["long heading ancestry\n\nbody words"])
            .unwrap_err();
        assert!(error.contains("7 tokens"), "{error}");
        assert!(error.contains("heading breadcrumbs"));
    }

    #[test]
    fn tokenizer_identity_is_stable_on_reload_and_changes_with_policy() {
        let tokenizer = tokenizer();
        let bytes = tokenizer.to_string(false).unwrap();
        let first = InputBudget::exact(tokenizer, 512).unwrap();
        let reloaded = InputBudget::exact(Tokenizer::from_bytes(&bytes).unwrap(), 512).unwrap();
        assert_eq!(first.identity(), reloaded.identity());
        let smaller = InputBudget::exact(Tokenizer::from_bytes(&bytes).unwrap(), 256).unwrap();
        assert_ne!(first.identity(), smaller.identity());
        let mut changed = Tokenizer::from_bytes(&bytes).unwrap();
        changed.with_normalizer(Some(tokenizers::normalizers::unicode::NFD));
        assert_ne!(
            first.identity(),
            InputBudget::exact(changed, 512).unwrap().identity()
        );
    }

    #[test]
    fn oversized_single_token_is_rejected_before_tokenization() {
        let policy = InputBudget::exact(tokenizer(), 5).unwrap();
        let text = "x".repeat(MAX_INPUT_BYTES + 1);
        let error = policy.validate([text.as_str()]).unwrap_err();
        assert!(error.contains("byte safety limit"), "{error}");
        assert!(error.contains("not truncated"));
    }

    fn assert_document_partition(
        budget: &InputBudget,
        prefix: &str,
        body: &str,
    ) -> Vec<Range<usize>> {
        let ranges = budget.document_ranges(prefix, body).unwrap();
        let mut covered = 0;
        for range in &ranges {
            assert_eq!(range.start, covered);
            assert!(range.end > range.start);
            assert!(body.is_char_boundary(range.start));
            assert!(body.is_char_boundary(range.end));
            budget
                .validate([format!("{prefix}{}", &body[range.clone()]).as_str()])
                .unwrap();
            covered = range.end;
        }
        assert_eq!(covered, body.len());
        ranges
    }

    #[test]
    fn document_byte_splits_fill_budget_and_preserve_utf8() {
        let budget = InputBudget::remote(Some(12), None).unwrap();
        let body = "你好🙂é漢字🙂";
        let ranges = assert_document_partition(&budget, "A\n\n", body);
        assert_eq!(ranges, vec![0..6, 6..15, 15..22]);
        assert!(budget.document_ranges("A\n\n", "").unwrap().is_empty());
        assert!(
            InputBudget::remote(Some(3), None)
                .unwrap()
                .document_ranges("", "🙂")
                .is_err()
        );
        let error = budget
            .document_ranges("oversized heading\n\n", body)
            .unwrap_err();
        assert!(error.contains("heading breadcrumbs"), "{error}");
        assert!(error.contains("No input was truncated"), "{error}");
    }

    #[test]
    fn document_splits_count_normalization_and_special_tokens_in_every_input() {
        let budget = InputBudget::exact(tokenizer(), 8).unwrap();
        let body = "ﷺ one two three four \nﷺ six seven";
        assert!(
            budget
                .validate([format!("heading\n\n{body}").as_str()])
                .is_err()
        );
        assert!(assert_document_partition(&budget, "heading\n\n", body).len() > 1);
        let tiny = InputBudget::exact(tokenizer(), 1).unwrap();
        let error = tiny.document_ranges("", "x").unwrap_err();
        assert!(error.contains("bounded input-budget search"), "{error}");
        assert!(error.contains("special tokens"), "{error}");
    }

    #[test]
    fn document_splits_try_nonmonotonic_normalized_prefixes() {
        let model = tokenizers::models::bpe::BPE::builder()
            .vocab_and_merges(
                [
                    ("f", 0),
                    ("i", 1),
                    ("x", 2),
                    ("q", 3),
                    ("ix", 4),
                    ("fix", 5),
                    ("ffix", 6),
                ]
                .map(|(text, id)| (text.to_string(), id)),
                [("i", "x"), ("f", "ix"), ("f", "fix")]
                    .into_iter()
                    .map(|(left, right)| (left.to_string(), right.to_string()))
                    .collect(),
            )
            .build()
            .unwrap();
        let mut tokenizer = Tokenizer::new(model);
        tokenizer.with_normalizer(Some(NFKC));
        let budget = InputBudget::exact(tokenizer, 1).unwrap();
        assert!(
            budget.validate(["ﬃ"]).is_err(),
            "singleton expands to three tokens"
        );
        budget.validate(["ﬃx"]).unwrap();
        assert_eq!(
            assert_document_partition(&budget, "", "ﬃxq"),
            vec![0..4, 4..5]
        );
    }

    #[test]
    fn document_splits_validate_composed_heading_instead_of_prefix_alone() {
        let model = tokenizers::models::bpe::BPE::builder()
            .vocab_and_merges(
                [
                    ("a", 0),
                    ("\n", 1),
                    ("x", 2),
                    ("\nx", 3),
                    ("\n\nx", 4),
                    ("a\n\nx", 5),
                ]
                .map(|(text, id)| (text.to_string(), id)),
                [("\n", "x"), ("\n", "\nx"), ("a", "\n\nx")]
                    .into_iter()
                    .map(|(left, right)| (left.to_string(), right.to_string()))
                    .collect(),
            )
            .build()
            .unwrap();
        let budget = InputBudget::exact(Tokenizer::new(model), 1).unwrap();
        assert!(budget.validate(["a\n\n"]).is_err());
        assert_eq!(
            assert_document_partition(&budget, "a\n\n", "x"),
            std::iter::once(0..1).collect::<Vec<_>>()
        );
    }

    #[test]
    fn document_splitting_bounds_tokenizer_work_and_output_parts() {
        let budget = InputBudget::exact(tokenizer(), 1).unwrap();
        let prefix = format!("{}\n\n", "a".repeat(10_000));
        let error = budget
            .document_ranges(&prefix, &"x".repeat(256))
            .unwrap_err();
        assert!(error.contains("tokenizer-work allowance"), "{error}");
        let byte_budget = InputBudget::remote(Some(1), None).unwrap();
        let error = byte_budget
            .document_ranges("", &"x".repeat(MAX_DOCUMENT_SPLIT_PARTS + 1))
            .unwrap_err();
        assert!(error.contains("part safety limit"), "{error}");
        let heading_budget = InputBudget::remote(Some(16_384), None).unwrap();
        let error = heading_budget
            .document_ranges(&"h".repeat(16_383), &"x".repeat(256))
            .unwrap_err();
        assert!(error.contains("output-size allowance"), "{error}");
    }

    #[test]
    fn document_splitting_handles_long_unbroken_inputs_above_byte_ceiling() {
        let budget = InputBudget::exact(tokenizer(), 5).unwrap();
        let body = "x".repeat(MAX_INPUT_BYTES + 7);
        let ranges = assert_document_partition(&budget, "", &body);
        assert_eq!(
            ranges,
            vec![0..MAX_INPUT_BYTES, MAX_INPUT_BYTES..body.len()]
        );
    }
}
