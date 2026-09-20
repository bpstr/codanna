//! Validate complete embedding inputs before inference, without altering evidence.
//!
//! Exact counts include normalization and model special tokens. A remote endpoint
//! without a supplied tokenizer uses an explicitly named byte-budget proxy; it is
//! deliberately conservative for common tokenizers, not a universal token bound.

use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use tokenizers::Tokenizer;

pub(crate) const INPUT_POLICY_VERSION: &str = "complete-input-v2";
const DEFAULT_REMOTE_BUDGET: usize = 8192;
const MAX_INPUT_BYTES: usize = 1024 * 1024;
const MAX_TOKENIZER_BYTES: u64 = 64 * 1024 * 1024;

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
            let (count, unit) = match &self.tokenizer {
                Some(tokenizer) => (
                    tokenizer
                        .encode(text, true)
                        .map_err(|error| {
                            format!("Cannot tokenize complete embedding input {index}: {error}")
                        })?
                        .len(),
                    "tokens (including special tokens)",
                ),
                None => (
                    text.len(),
                    "UTF-8 bytes (token-budget proxy; exact tokenizer unavailable)",
                ),
            };
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
}
