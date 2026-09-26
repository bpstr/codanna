//! Pinned-reader graph queries: bounded pages, bulk symbol hydration and BFS frontiers.
use super::{Document, DocumentIndex};
use crate::{
    RelationKind, Relationship, Symbol, SymbolId,
    storage::{StorageError, StorageResult},
};
use std::collections::{HashMap, HashSet};
use tantivy::{
    Searcher, Term,
    collector::{Count, DocSetCollector, TopDocs},
    query::{BooleanQuery, Occur, Query, TermQuery, TermSetQuery},
    schema::{IndexRecordOption, Value},
};

#[path = "graph_scope.rs"]
mod endpoint_scope;

pub type GraphEdge = (SymbolId, SymbolId, Relationship);
#[derive(Debug)]
pub struct RelationshipPage {
    pub edges: Vec<GraphEdge>,
    pub total: usize,
    pub next_offset: Option<usize>,
}

/// A page cursor is an offset within this pinned view, not a global live-index
/// cursor. Keep this view for the complete traversal; concurrent commits cannot
/// change its pagination or mix symbol hydration with a different generation.
pub struct GraphView<'a> {
    index: &'a DocumentIndex,
    searcher: Searcher,
    #[cfg(test)]
    queries: std::cell::Cell<usize>,
}
impl DocumentIndex {
    pub fn graph_view(&self) -> GraphView<'_> {
        GraphView {
            index: self,
            searcher: self.reader.searcher(),
            #[cfg(test)]
            queries: std::cell::Cell::new(0),
        }
    }
    /// Hydrate each distinct ID with one query per 512 IDs, preserving the
    /// caller's ordering (including duplicates) and omitting missing IDs.
    pub fn find_symbols_by_ids(&self, ids: &[SymbolId]) -> StorageResult<Vec<Symbol>> {
        self.graph_view().symbols(ids)
    }
}
impl GraphView<'_> {
    fn counted(&self) {
        #[cfg(test)]
        self.queries.set(self.queries.get() + 1);
    }
    fn query(
        &self,
        ids: &[SymbolId],
        incoming: bool,
        kinds: &[RelationKind],
    ) -> StorageResult<BooleanQuery> {
        if ids.is_empty() || ids.len() > 512 || kinds.is_empty() || kinds.len() > 12 {
            return Err(StorageError::General(
                "graph query requires 1..512 IDs and 1..12 kinds".into(),
            ));
        }
        let s = &self.index.schema;
        Ok(BooleanQuery::new(vec![
            (
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(s.doc_type, "relationship"),
                    IndexRecordOption::Basic,
                )) as Box<dyn Query>,
            ),
            (
                Occur::Must,
                Box::new(TermSetQuery::new(ids.iter().map(|id| {
                    Term::from_field_u64(
                        if incoming {
                            s.to_symbol_id
                        } else {
                            s.from_symbol_id
                        },
                        u64::from(id.value()),
                    )
                }))),
            ),
            (
                Occur::Must,
                Box::new(TermSetQuery::new(kinds.iter().map(|kind| {
                    Term::from_field_text(s.relation_kind, &format!("{kind:?}"))
                }))),
            ),
        ]))
    }
    fn edge(&self, address: tantivy::DocAddress) -> StorageResult<GraphEdge> {
        let doc = self.searcher.doc::<Document>(address)?;
        let s = &self.index.schema;
        let id = |field| {
            doc.get_first(field)
                .and_then(|v| v.as_u64())
                .and_then(|n| u32::try_from(n).ok())
                .and_then(SymbolId::new)
                .ok_or_else(|| StorageError::General("invalid graph endpoint ID".into()))
        };
        let kind = doc
            .get_first(s.relation_kind)
            .and_then(|v| v.as_str())
            .and_then(super::query::relation_kind_from_stored)
            .ok_or_else(|| StorageError::General("invalid stored relationship kind".into()))?;
        let mut relationship = Relationship::new(kind);
        if let Some(metadata) = self.index.metadata_for_relationship_doc(&doc) {
            relationship = relationship.with_metadata(metadata);
        }
        Ok((id(s.from_symbol_id)?, id(s.to_symbol_id)?, relationship))
    }
    pub fn relationships_page(
        &self,
        ids: &[SymbolId],
        incoming: bool,
        kinds: &[RelationKind],
        offset: usize,
        limit: usize,
    ) -> StorageResult<RelationshipPage> {
        if !(1..=1000).contains(&limit) || offset > 100_000 {
            return Err(StorageError::General(
                "graph page limit must be 1..1000, offset at most 100000".into(),
            ));
        }
        let query = self.query(ids, incoming, kinds)?;
        self.counted();
        let (total, docs) = self.searcher.search(
            &query,
            &(
                Count,
                TopDocs::with_limit(limit)
                    .and_offset(offset)
                    .order_by_score(),
            ),
        )?;
        let edges = docs
            .into_iter()
            .map(|(_, addr)| self.edge(addr))
            .collect::<StorageResult<Vec<_>>>()?;
        let next = offset + edges.len();
        Ok(RelationshipPage {
            edges,
            total,
            next_offset: (next < total).then_some(next),
        })
    }
    /// Complete legacy enumeration. A caller supplying a budget gets an error
    /// before hydration when that budget is exceeded; never a partial success.
    pub fn relationships(
        &self,
        ids: &[SymbolId],
        incoming: bool,
        kinds: &[RelationKind],
        budget: Option<usize>,
    ) -> StorageResult<Vec<GraphEdge>> {
        let query = self.query(ids, incoming, kinds)?;
        if let Some(max) = budget {
            if max == 0 {
                self.counted();
                return if self.searcher.search(&query, &Count)? == 0 {
                    Ok(vec![])
                } else {
                    Err(StorageError::GraphBudgetExceeded {
                        resource: "edge",
                        limit: max,
                    })
                };
            }
            if max > 100_000 {
                return Err(StorageError::General(
                    "graph edge budget exceeds server maximum".into(),
                ));
            }
            self.counted();
            let (count, docs) = self
                .searcher
                .search(&query, &(Count, TopDocs::with_limit(max).order_by_score()))?;
            if count > max {
                return Err(StorageError::GraphBudgetExceeded {
                    resource: "edge",
                    limit: max,
                });
            }
            docs.into_iter().map(|(_, addr)| self.edge(addr)).collect()
        } else {
            self.counted();
            let mut docs: Vec<_> = self
                .searcher
                .search(&query, &DocSetCollector)?
                .into_iter()
                .collect();
            docs.sort_unstable();
            docs.into_iter().map(|addr| self.edge(addr)).collect()
        }
    }
    pub fn symbols(&self, ids: &[SymbolId]) -> StorageResult<Vec<Symbol>> {
        let mut unique: Vec<_> = ids
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        unique.sort_unstable_by_key(|id| id.value());
        let mut found = HashMap::with_capacity(unique.len());
        for chunk in unique.chunks(512) {
            let query = BooleanQuery::new(vec![
                (
                    Occur::Must,
                    Box::new(TermSetQuery::new(chunk.iter().map(|id| {
                        Term::from_field_u64(self.index.schema.symbol_id, u64::from(id.value()))
                    }))) as Box<dyn Query>,
                ),
                (
                    Occur::Must,
                    Box::new(TermQuery::new(
                        Term::from_field_text(self.index.schema.doc_type, "symbol"),
                        IndexRecordOption::Basic,
                    )) as Box<dyn Query>,
                ),
            ]);
            self.counted();
            let mut docs: Vec<_> = self
                .searcher
                .search(&query, &DocSetCollector)?
                .into_iter()
                .collect();
            docs.sort_unstable();
            for addr in docs {
                let doc = self.searcher.doc::<Document>(addr)?;
                let symbol = self.index.document_to_symbol(&doc)?;
                found.entry(symbol.id).or_insert(symbol);
            }
        }
        Ok(ids.iter().filter_map(|id| found.get(id).cloned()).collect())
    }
    pub fn impact(
        &self,
        start: SymbolId,
        depth: usize,
        budget: Option<(usize, usize)>,
    ) -> StorageResult<Vec<SymbolId>> {
        if budget.is_some_and(|(nodes, edges)| nodes == 0 || nodes > 10_000 || edges > 100_000) {
            return Err(StorageError::General(
                "invalid graph traversal budget".into(),
            ));
        }
        let mut visited = HashSet::from([start]);
        let mut frontier = vec![start];
        let mut used_edges = 0usize;
        for _ in 0..depth {
            let mut next = Vec::new();
            for chunk in frontier.chunks(512) {
                let edges = self.relationships(
                    chunk,
                    true,
                    &[
                        RelationKind::Calls,
                        RelationKind::References,
                        RelationKind::Uses,
                        RelationKind::Implements,
                        RelationKind::Extends,
                    ],
                    budget.map(|(_, max)| max.saturating_sub(used_edges)),
                )?;
                used_edges += edges.len();
                for (from, _, _) in edges {
                    if visited.insert(from) {
                        if let Some((max, _)) = budget
                            && visited.len() - 1 > max
                        {
                            return Err(StorageError::GraphBudgetExceeded {
                                resource: "node",
                                limit: max,
                            });
                        }
                        next.push(from);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            next.sort_unstable_by_key(|id| id.value());
            frontier = next;
        }
        visited.remove(&start);
        let mut result: Vec<_> = visited.into_iter().collect();
        result.sort_unstable_by_key(|id| id.value());
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hardening_final_graph_batches_hydration_and_frontier_without_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let settings = crate::Settings::default();
        let index = DocumentIndex::new(dir.path(), &settings).unwrap();
        index.start_batch().unwrap();
        let root = SymbolId::new(1).unwrap();
        for id in 1..=1201 {
            let symbol = Symbol::new(
                SymbolId::new(id).unwrap(),
                format!("node{id}"),
                crate::SymbolKind::Function,
                crate::FileId::new(1).unwrap(),
                crate::Range::new(id, 0, id, 1),
            );
            index.index_symbol(&symbol, "src/fixture.rs").unwrap();
            if id > 1 {
                index
                    .store_relationship(symbol.id, root, &Relationship::new(RelationKind::Calls))
                    .unwrap();
            }
        }
        index.commit_batch().unwrap();
        let graph = index.graph_view();
        let impacted = graph.impact(root, 1, None).unwrap();
        assert_eq!(impacted.len(), 1200);
        assert_eq!(graph.queries.get(), 1);
        assert_eq!(graph.symbols(&impacted).unwrap().len(), 1200);
        assert_eq!(
            graph.queries.get(),
            4,
            "one graph search and three hydration chunks, not 1200 point fetches"
        );
        assert!(graph.impact(root, 2, Some((1000, 20_000))).is_err());
        assert!(
            graph
                .relationships(&[root], true, &[RelationKind::Calls], Some(1000))
                .is_err()
        );
        assert!(
            graph
                .relationships_page(&[root], true, &[RelationKind::Calls], 0, 0)
                .is_err()
        );
        let first = graph
            .relationships_page(&[root], true, &[RelationKind::Calls], 0, 1000)
            .unwrap();
        assert_eq!(first.total, 1200);
        assert_eq!(first.next_offset, Some(1000));
        index.start_batch().unwrap();
        index
            .store_relationship(
                SymbolId::new(9000).unwrap(),
                root,
                &Relationship::new(RelationKind::Calls),
            )
            .unwrap();
        index.commit_batch().unwrap();
        let second = graph
            .relationships_page(
                &[root],
                true,
                &[RelationKind::Calls],
                first.next_offset.unwrap(),
                1000,
            )
            .unwrap();
        assert_eq!(second.edges.len(), 200);
        assert_eq!(second.total, 1200);
        assert_eq!(second.next_offset, None);
        let ids: HashSet<_> = first
            .edges
            .iter()
            .chain(&second.edges)
            .map(|(id, _, _)| *id)
            .collect();
        assert_eq!(ids.len(), 1200);
        assert_eq!(
            index
                .graph_view()
                .relationships_page(&[root], true, &[RelationKind::Calls], 0, 1)
                .unwrap()
                .total,
            1201
        );
    }
}
