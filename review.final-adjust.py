from pathlib import Path


def replace(path, before, after):
    p = Path(path)
    text = p.read_text()
    assert text.count(before) == 1, (path, before[:80], text.count(before))
    p.write_text(text.replace(before, after))


for name in ['replaced_after_preflight', 'caller', 'target']:
    replace('src/indexing/walker.rs', f'.find_symbols_by_name("{name}")', f'.find_symbols_by_name("{name}", None)')
replace('src/indexing/facade.rs', 'pub type FacadeResult<T> = Result<T, IndexError>;', '''pub type FacadeResult<T> = Result<T, IndexError>;

/// A hydrated adjacent symbol and its optional edge metadata.
pub type GraphNeighbor = (Symbol, Option<crate::relationship::RelationshipMetadata>);
/// Visible neighbors followed by the total matching edge count.
pub type GraphNeighborPreview = (Vec<GraphNeighbor>, usize);''')
replace('src/indexing/facade.rs', '''    ) -> FacadeResult<(
        Vec<(Symbol, Option<crate::relationship::RelationshipMetadata>)>,
        usize,
    )> {''', '    ) -> FacadeResult<GraphNeighborPreview> {')
