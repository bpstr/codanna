//! Graph endpoint filtering uses the same registered-file scope as lexical search.
use super::GraphView;
use crate::storage::{StorageError, StorageResult};
use crate::{Symbol, SymbolId};
use std::collections::HashMap;
use tantivy::collector::DocSetCollector;
use tantivy::query::{BooleanQuery, Occur, Query, TermQuery, TermSetQuery};
use tantivy::schema::IndexRecordOption;
use tantivy::{TantivyDocument, Term};

impl GraphView<'_> {
    /// Instance-local identity of the pinned graph reader, not source freshness.
    pub fn reader_generation(&self) -> u64 {
        self.searcher.generation().generation_id()
    }

    /// Hydrate at most 512 endpoint IDs within an explicit workspace subtree.
    ///
    /// Registration lookup, filtering and hydration share this view's Searcher.
    /// Missing registrations and external roots cannot fall back to path display
    /// shortening or unscoped name lookup. Input order and duplicates are kept.
    pub fn symbols_scoped(&self, ids: &[SymbolId], prefix: &str) -> StorageResult<Vec<Symbol>> {
        if ids.len() > 512 {
            return Err(StorageError::General(
                "scoped graph hydration accepts at most 512 IDs".into(),
            ));
        }
        let file_terms = self.index.file_scope_terms(&self.searcher, prefix)?;
        if ids.is_empty() || file_terms.is_empty() {
            return Ok(Vec::new());
        }
        let s = &self.index.schema;
        let query = BooleanQuery::new(vec![
            (
                Occur::Must,
                Box::new(TermSetQuery::new(ids.iter().map(|id| {
                    Term::from_field_u64(s.symbol_id, u64::from(id.value()))
                }))) as Box<dyn Query>,
            ),
            (
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(s.doc_type, "symbol"),
                    IndexRecordOption::Basic,
                )),
            ),
            (Occur::Must, Box::new(TermSetQuery::new(file_terms))),
        ]);
        self.counted();
        let mut addresses: Vec<_> = self
            .searcher
            .search(&query, &DocSetCollector)?
            .into_iter()
            .collect();
        addresses.sort_unstable();
        let mut found = HashMap::with_capacity(ids.len());
        for address in addresses {
            let doc: TantivyDocument = self.searcher.doc(address)?;
            let symbol = self.index.document_to_symbol(&doc)?;
            found.entry(symbol.id).or_insert(symbol);
        }
        Ok(ids.iter().filter_map(|id| found.get(id).cloned()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexing::pipeline::FileRegistration;
    use crate::parsing::LanguageId;
    use crate::storage::DocumentIndex;
    use crate::{FileId, Range, Settings, SymbolKind};

    #[test]
    fn scoped_graph_registration_and_hydration_use_one_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let index = DocumentIndex::new(temp.path(), &Settings::default()).unwrap();
        let id = SymbolId::new(1).unwrap();
        let symbol = Symbol::new(
            id,
            "owner",
            SymbolKind::Function,
            FileId::new(1).unwrap(),
            Range::new(0, 0, 0, 20),
        );
        index.start_batch().unwrap();
        index.index_symbol(&symbol, "src/active.rs").unwrap();
        index.commit_batch().unwrap();
        let old = index.graph_view();
        assert_eq!(old.symbols(&[id]).unwrap().len(), 1);
        assert!(old.symbols_scoped(&[id], "src").unwrap().is_empty());
        index.start_batch().unwrap();
        index
            .store_file_registration(&FileRegistration {
                path: "src/active.rs".into(),
                file_id: FileId::new(1).unwrap(),
                content_hash: "fixture".into(),
                language_id: LanguageId::new("rust"),
                timestamp: 0,
                mtime: 0,
            })
            .unwrap();
        index.commit_batch().unwrap();
        let new = index.graph_view();
        assert_ne!(old.reader_generation(), new.reader_generation());
        assert_eq!(new.symbols_scoped(&[id, id], "src").unwrap().len(), 2);
        assert!(old.symbols_scoped(&[id], "src").unwrap().is_empty());
        assert_eq!(new.symbols_scoped(&[id], ".").unwrap().len(), 1);
        index.start_batch().unwrap();
        index.remove_file_documents("src/active.rs").unwrap();
        index.commit_batch().unwrap();
        assert!(
            index
                .graph_view()
                .symbols_scoped(&[id], "src")
                .unwrap()
                .is_empty()
        );
        assert_eq!(new.symbols_scoped(&[id], "src").unwrap().len(), 1);
    }

    #[test]
    fn scoped_graph_validates_limits_and_invalid_scopes_even_for_empty_ids() {
        let temp = tempfile::tempdir().unwrap();
        let index = DocumentIndex::new(temp.path(), &Settings::default()).unwrap();
        let view = index.graph_view();
        let id = SymbolId::new(1).unwrap();
        assert!(view.symbols_scoped(&[id; 513], ".").is_err());
        for invalid in ["", "..", "/tmp", r"C:\external", "src/../elsewhere"] {
            assert!(view.symbols_scoped(&[], invalid).is_err(), "{invalid}");
        }
        assert!(view.symbols_scoped(&[], "src").unwrap().is_empty());
    }
}
