//! Document chunking strategies.
//!
//! Provides the `Chunker` trait and implementations for splitting documents
//! into chunks suitable for embedding.

use super::config::ValidatedChunkingConfig;

/// A raw chunk before being assigned IDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawChunk {
    /// Byte range in the source document (start, end).
    pub byte_range: (usize, usize),

    /// The text content of this chunk.
    pub content: String,

    /// Heading hierarchy context (e.g., ["Chapter 1", "Section 1.2"]).
    pub heading_context: Vec<String>,
}

impl RawChunk {
    /// Create a new raw chunk.
    pub fn new(byte_range: (usize, usize), content: String, heading_context: Vec<String>) -> Self {
        Self {
            byte_range,
            content,
            heading_context,
        }
    }

    /// Get character count.
    pub fn char_count(&self) -> usize {
        self.content.chars().count()
    }
}

/// Trait for document chunking strategies.
pub trait Chunker: Send + Sync {
    /// Split document content into chunks.
    fn chunk(&self, content: &str, config: &ValidatedChunkingConfig) -> Vec<RawChunk>;
}

/// Hybrid chunker: paragraph-based with size constraints.
///
/// Algorithm:
/// 1. Extract heading positions for context
/// 2. Split by paragraphs (double newline)
/// 3. Merge small paragraphs (< min_chunk_chars)
/// 4. Split large chunks with sliding window + overlap
/// 5. Attach heading context to each chunk
#[derive(Debug, Default)]
pub struct HybridChunker;

impl HybridChunker {
    /// Create a new hybrid chunker.
    pub fn new() -> Self {
        Self
    }
}

/// A heading found in the document.
#[derive(Debug, Clone)]
struct Heading {
    /// Level (1-6 for H1-H6).
    level: u8,
    /// Text of the heading.
    text: String,
    /// Byte position where the heading's source line starts.
    start_byte: usize,
}

impl Chunker for HybridChunker {
    fn chunk(&self, content: &str, config: &ValidatedChunkingConfig) -> Vec<RawChunk> {
        if content.is_empty() {
            return Vec::new();
        }

        // Step 1: Extract headings for context
        let headings = extract_headings(content);

        // Step 2: Split by paragraphs
        let paragraphs = split_paragraphs(content, &headings);

        // Step 3: Merge small paragraphs
        let merged = merge_small_paragraphs(content, paragraphs, config.min_chunk_chars, &headings);

        // Step 4: Split large chunks with sliding window
        let split = split_large_chunks(merged, config.max_chunk_chars, config.overlap_chars);

        // Step 5: Attach heading context
        attach_heading_context(split, &headings)
    }
}

/// A paragraph with its byte range.
#[derive(Debug, Clone)]
struct Paragraph<'a> {
    byte_range: (usize, usize),
    content: &'a str,
}

/// Extract ATX headings in source order, excluding fenced and indented code.
fn extract_headings(content: &str) -> Vec<Heading> {
    let mut headings = Vec::new();
    let mut offset = 0;
    let mut fence: Option<(u8, usize)> = None;

    for raw_line in content.split_inclusive('\n') {
        let line_start = offset;
        offset += raw_line.len();
        let line = raw_line.trim_end_matches(['\r', '\n']);
        let indentation = line.bytes().take_while(|byte| *byte == b' ').count();
        if indentation > 3 || line.starts_with('\t') {
            continue;
        }
        let line = &line[indentation..];
        let marker = fence_marker(line);
        if let Some((open_marker, open_length)) = fence {
            if marker.is_some_and(|(marker, length, suffix)| {
                marker == open_marker && length >= open_length && suffix.trim().is_empty()
            }) {
                fence = None;
            }
            continue;
        }
        if let Some((marker, length, suffix)) = marker {
            // A backtick in a backtick fence's info string is not an opener.
            if marker != b'`' || !suffix.contains('`') {
                fence = Some((marker, length));
            }
            continue;
        }

        let level = line.bytes().take_while(|byte| *byte == b'#').count();
        if !(1..=6).contains(&level) {
            continue;
        }
        let rest = &line[level..];
        if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
            continue;
        }
        let mut title = rest.trim();
        let without_closing_hashes = title.trim_end_matches('#');
        if without_closing_hashes.is_empty()
            || without_closing_hashes.ends_with(char::is_whitespace)
        {
            title = without_closing_hashes.trim_end();
        }
        headings.push(Heading {
            level: level as u8,
            text: title.to_string(),
            start_byte: line_start,
        });
    }
    headings
}

fn fence_marker(line: &str) -> Option<(u8, usize, &str)> {
    let marker = *line.as_bytes().first()?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let length = line.bytes().take_while(|byte| *byte == marker).count();
    (length >= 3).then_some((marker, length, &line[length..]))
}

/// Keep a trimmed paragraph's range tied to the same source bytes as its text.
fn paragraph_at(content: &str, start: usize, end: usize) -> Option<Paragraph<'_>> {
    let raw = &content[start..end];
    let leading = raw.len() - raw.trim_start().len();
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    let start = start + leading;
    Some(Paragraph {
        byte_range: (start, start + text.len()),
        content: text,
    })
}

/// Split on blank lines (LF or CRLF) and on heading boundaries.
fn split_paragraphs<'a>(content: &'a str, headings: &[Heading]) -> Vec<Paragraph<'a>> {
    let mut paragraphs = Vec::new();
    let mut para_start = 0;
    let mut offset = 0;
    let mut next_heading = headings.iter().peekable();
    for line in content.split_inclusive('\n') {
        let starts_heading = next_heading
            .peek()
            .is_some_and(|heading| heading.start_byte == offset);
        if starts_heading {
            next_heading.next();
            if let Some(paragraph) = paragraph_at(content, para_start, offset) {
                paragraphs.push(paragraph);
            }
            para_start = offset;
        }
        if line.trim().is_empty() {
            if let Some(paragraph) = paragraph_at(content, para_start, offset) {
                paragraphs.push(paragraph);
            }
            para_start = offset + line.len();
        }
        offset += line.len();
    }
    if let Some(paragraph) = paragraph_at(content, para_start, content.len()) {
        paragraphs.push(paragraph);
    }
    paragraphs
}

/// Merge small paragraphs within one heading scope, preserving all intervening bytes.
fn merge_small_paragraphs<'a>(
    content: &'a str,
    paragraphs: Vec<Paragraph<'a>>,
    min_chars: usize,
    headings: &[Heading],
) -> Vec<Paragraph<'a>> {
    if paragraphs.is_empty() {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut iter = paragraphs.into_iter();
    let mut current = iter.next().unwrap();

    for para in iter {
        let same_section = headings
            .partition_point(|heading| heading.start_byte <= current.byte_range.0)
            == headings.partition_point(|heading| heading.start_byte <= para.byte_range.0);
        if current.content.chars().count() < min_chars && same_section {
            current.byte_range.1 = para.byte_range.1;
            current.content = &content[current.byte_range.0..current.byte_range.1];
        } else {
            result.push(current);
            current = para;
        }
    }

    // Don't forget the last one
    result.push(current);
    result
}

/// Split large paragraphs with sliding window and overlap.
fn split_large_chunks(
    paragraphs: Vec<Paragraph<'_>>,
    max_chars: usize,
    overlap_chars: usize,
) -> Vec<Paragraph<'_>> {
    let mut result = Vec::new();

    for para in paragraphs {
        let char_count = para.content.chars().count();

        if char_count <= max_chars {
            result.push(para);
        } else {
            // Split with sliding window
            let boundaries: Vec<usize> = para
                .content
                .char_indices()
                .map(|(index, _)| index)
                .chain(std::iter::once(para.content.len()))
                .collect();
            let step = max_chars.saturating_sub(overlap_chars).max(1);

            let mut char_start = 0;
            while char_start < char_count {
                let char_end = char_start.saturating_add(max_chars).min(char_count);
                let byte_start = boundaries[char_start];
                let byte_end = boundaries[char_end];

                result.push(Paragraph {
                    byte_range: (para.byte_range.0 + byte_start, para.byte_range.0 + byte_end),
                    content: &para.content[byte_start..byte_end],
                });

                if char_end >= char_count {
                    break;
                }
                char_start += step;
            }
        }
    }

    result
}

/// Attach heading context to each chunk.
fn attach_heading_context(paragraphs: Vec<Paragraph<'_>>, headings: &[Heading]) -> Vec<RawChunk> {
    let mut next_heading = headings.iter().peekable();
    let mut hierarchy: Vec<&Heading> = Vec::new();
    paragraphs
        .into_iter()
        .map(|para| {
            while next_heading
                .peek()
                .is_some_and(|heading| heading.start_byte <= para.byte_range.0)
            {
                let heading = next_heading.next().expect("peeked heading exists");
                while hierarchy
                    .last()
                    .is_some_and(|parent| parent.level >= heading.level)
                {
                    hierarchy.pop();
                }
                hierarchy.push(heading);
            }
            let context = hierarchy
                .iter()
                .filter(|heading| !heading.text.is_empty())
                .map(|heading| heading.text.clone())
                .collect();

            RawChunk::new(para.byte_range, para.content.to_string(), context)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::config::ChunkingConfig;
    use super::*;

    fn default_config() -> ValidatedChunkingConfig {
        ChunkingConfig {
            min_chunk_chars: 50,
            max_chunk_chars: 200,
            overlap_chars: 20,
            ..Default::default()
        }
        .try_into()
        .unwrap()
    }

    #[test]
    fn test_empty_content() {
        let chunker = HybridChunker::new();
        let chunks = chunker.chunk("", &default_config());
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_single_paragraph() {
        let chunker = HybridChunker::new();
        let content = "This is a single paragraph with enough text to be meaningful.";
        let chunks = chunker.chunk(content, &default_config());

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, content);
    }

    #[test]
    fn test_multiple_paragraphs() {
        let chunker = HybridChunker::new();
        let content =
            "First paragraph with some content here.\n\nSecond paragraph with more content here.";
        let config = ChunkingConfig {
            min_chunk_chars: 10,
            max_chunk_chars: 200,
            overlap_chars: 5,
            ..Default::default()
        };
        let config = ValidatedChunkingConfig::try_from(config).unwrap();
        let chunks = chunker.chunk(content, &config);

        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].content.contains("First"));
        assert!(chunks[1].content.contains("Second"));
    }

    #[test]
    fn test_merges_small_paragraphs() {
        let chunker = HybridChunker::new();
        let content =
            "Tiny.\n\nAlso tiny.\n\nThis is a longer paragraph that should stand on its own.";
        let config = ChunkingConfig {
            min_chunk_chars: 50,
            max_chunk_chars: 500,
            overlap_chars: 20,
            ..Default::default()
        };
        let config = ValidatedChunkingConfig::try_from(config).unwrap();
        let chunks = chunker.chunk(content, &config);

        // "Tiny." and "Also tiny." should be merged
        // The longer paragraph should be separate
        assert!(chunks.len() <= 2);
        assert!(chunks[0].content.contains("Tiny"));
    }

    #[test]
    fn test_splits_large_paragraph() {
        let chunker = HybridChunker::new();
        // Create a paragraph larger than max_chunk_chars
        let content = "word ".repeat(100); // ~500 chars
        let config = ChunkingConfig {
            min_chunk_chars: 21,
            max_chunk_chars: 100,
            overlap_chars: 20,
            ..Default::default()
        };
        let config = ValidatedChunkingConfig::try_from(config).unwrap();
        let chunks = chunker.chunk(&content, &config);

        // Should be split into multiple chunks
        assert!(chunks.len() > 1);

        // Each chunk should be at most max_chunk_chars
        for chunk in &chunks {
            assert!(chunk.char_count() <= config.max_chunk_chars);
        }
    }

    #[test]
    fn test_heading_context() {
        let chunker = HybridChunker::new();
        let content = r#"# Chapter 1

Introduction paragraph here.

## Section 1.1

Content in section 1.1.

## Section 1.2

Content in section 1.2.

# Chapter 2

Content in chapter 2."#;

        let config = ChunkingConfig {
            min_chunk_chars: 11,
            max_chunk_chars: 500,
            overlap_chars: 10,
            ..Default::default()
        };
        let config = ValidatedChunkingConfig::try_from(config).unwrap();
        let chunks = chunker.chunk(content, &config);

        // Find the chunk with "section 1.2" content
        let section_12_chunk = chunks
            .iter()
            .find(|c| c.content.contains("Content in section 1.2"))
            .expect("Should find section 1.2 chunk");

        // Should have heading context
        assert!(!section_12_chunk.heading_context.is_empty());
        assert!(
            section_12_chunk
                .heading_context
                .iter()
                .any(|h| h.contains("Chapter 1"))
        );
    }

    #[test]
    fn test_byte_ranges_valid() {
        let chunker = HybridChunker::new();
        let content = "First paragraph.\n\nSecond paragraph.";
        let chunks = chunker.chunk(content, &default_config());

        for chunk in &chunks {
            let (start, end) = chunk.byte_range;
            assert!(start <= end);
            assert!(end <= content.len());
        }
    }

    #[test]
    fn test_overlap_between_split_chunks() {
        let chunker = HybridChunker::new();
        let content = "The quick brown fox jumps over the lazy dog. ".repeat(20);
        let config = ChunkingConfig {
            min_chunk_chars: 40,
            max_chunk_chars: 100,
            overlap_chars: 30,
            ..Default::default()
        };
        let config = ValidatedChunkingConfig::try_from(config).unwrap();
        let chunks = chunker.chunk(&content, &config);

        // Verify multiple chunks were created due to size limit
        assert!(
            chunks.len() > 1,
            "Large content should be split into multiple chunks"
        );
    }

    #[test]
    fn heading_scopes_stay_in_source_order_across_crlf_and_small_sections() {
        let source = "# Alpha\r\n\r\none\r\n## Beta\r\npayload\r\n# Gamma\r\n\r\ntail";
        let config = ValidatedChunkingConfig::try_from(ChunkingConfig {
            min_chunk_chars: 50,
            max_chunk_chars: 500,
            overlap_chars: 0,
            ..Default::default()
        })
        .unwrap();
        let chunks = HybridChunker::new().chunk(source, &config);
        assert_eq!(
            chunks.len(),
            3,
            "a small section must not absorb the next heading"
        );
        assert_eq!(chunks[0].heading_context, ["Alpha"]);
        assert_eq!(chunks[1].heading_context, ["Alpha", "Beta"]);
        assert_eq!(chunks[2].heading_context, ["Gamma"]);
        for chunk in chunks {
            assert_eq!(
                source.get(chunk.byte_range.0..chunk.byte_range.1),
                Some(chunk.content.as_str())
            );
        }
    }

    #[test]
    fn fenced_indented_and_quoted_hashes_do_not_change_document_context() {
        let config = ValidatedChunkingConfig::try_from(ChunkingConfig {
            min_chunk_chars: 1,
            max_chunk_chars: 500,
            overlap_chars: 0,
            ..Default::default()
        })
        .unwrap();
        for fence in [
            "```sh\n# comment\n```",
            "~~~~ rust\n# comment\n~~~\n# still code\n~~~~",
            "````sh\n```\n# still code\n````",
        ] {
            let source =
                format!("# Real\n\n{fence}\n\n    # indented\n\n#hashtag\n\n> # quoted\n\npayload");
            let chunks = HybridChunker::new().chunk(&source, &config);
            let payload = chunks
                .iter()
                .find(|chunk| chunk.content == "payload")
                .unwrap();
            assert_eq!(payload.heading_context, ["Real"], "fence: {fence}");
        }
    }

    #[test]
    fn headings_keep_skipped_levels_and_strip_only_separated_closing_hashes() {
        let source =
            "# Root ###\n\n### C#\n\npayload\n\n## Peer #\n\nother\n\n####### invalid\n\nlast";
        let config = ValidatedChunkingConfig::try_from(ChunkingConfig {
            min_chunk_chars: 1,
            max_chunk_chars: 500,
            overlap_chars: 0,
            ..Default::default()
        })
        .unwrap();
        let chunks = HybridChunker::new().chunk(source, &config);
        let context = |text| {
            &chunks
                .iter()
                .find(|chunk| chunk.content == text)
                .unwrap()
                .heading_context
        };
        assert_eq!(context("payload"), &["Root", "C#"]);
        assert_eq!(context("other"), &["Root", "Peer"]);
        assert_eq!(context("last"), &["Root", "Peer"]);
    }

    #[test]
    fn whitespace_merging_and_unicode_windows_reproduce_exact_source_ranges() {
        let source = format!(
            "\u{2003}first   \r\n \r\n  second{}\t  ",
            "🙂漢é".repeat(30)
        );
        let config = ValidatedChunkingConfig::try_from(ChunkingConfig {
            min_chunk_chars: 30,
            max_chunk_chars: 40,
            overlap_chars: 10,
            ..Default::default()
        })
        .unwrap();
        let chunks = HybridChunker::new().chunk(&source, &config);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert_eq!(
                source.get(chunk.byte_range.0..chunk.byte_range.1),
                Some(chunk.content.as_str())
            );
            assert!(chunk.char_count() <= 40);
        }
        for pair in chunks.windows(2) {
            let previous: Vec<_> = pair[0].content.chars().rev().take(10).collect();
            let mut next: Vec<_> = pair[1].content.chars().take(10).collect();
            next.reverse();
            assert_eq!(
                previous, next,
                "overlap must count Unicode characters without corrupting bytes"
            );
        }
    }
}
