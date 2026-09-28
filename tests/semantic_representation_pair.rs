//! Body/comment input comparison uses only the parser/capture stage.
use codanna::indexing::{calculate_hash, pipeline::{FileContent, stages::parse::{ParseStage, init_parser_cache}}};
use codanna::symbol_representation::{CodeEmbeddingPolicy, MAX_FILE_SOURCE_BYTES, MAX_SYMBOL_SOURCE_BYTES};
use std::sync::Arc;

#[test]
fn body_v2_retains_behavior_missing_from_documentation_without_embedding() {
    let temp = tempfile::tempdir().unwrap();
    let source = "/** Merge the latest page. */\nexport function mergePage(current: number[], incoming: number[]) { return [...new Set([...current, ...incoming])]; }\nexport function undocumented(value: number) { return value + 1; }\n";
    let mut settings = codanna::Settings {
        workspace_root: Some(temp.path().to_owned()), ..Default::default()
    };
    assert_eq!(settings.semantic_search.code_representation, CodeEmbeddingPolicy::DocComment);
    settings.semantic_search.enabled = true;
    settings.semantic_search.code_representation = CodeEmbeddingPolicy::SymbolBodyV2;
    let settings = Arc::new(settings);
    init_parser_cache(settings.clone());
    let parsed = ParseStage::new(settings).parse_file(FileContent::new(
        temp.path().join("page.ts"), source.into(), calculate_hash(source)
    )).unwrap();
    let mut retained = 0;
    for name in ["mergePage", "undocumented"] {
        let matches: Vec<_> = parsed.raw_symbols.iter().filter(|symbol| symbol.name.as_ref() == name).collect();
        assert_eq!(matches.len(), 1);
        let symbol = matches[0];
        let input = symbol.embedding_source.as_ref().expect("body capture");
        let body: String = input.fragments.iter().map(|fragment| fragment.text.as_str()).collect();
        assert!(body.contains(if name == "mergePage" { "new Set" } else { "value + 1" }));
        if name == "mergePage" {
            assert!(!symbol.doc_comment.as_deref().unwrap().contains("new Set"));
        } else {
            assert!(!CodeEmbeddingPolicy::DocComment.eligible(symbol.kind, symbol.doc_comment.is_some()));
        }
        assert!(input.retained_bytes() <= MAX_SYMBOL_SOURCE_BYTES);
        retained += input.retained_bytes();
    }
    assert!(retained <= MAX_FILE_SOURCE_BYTES);
}
