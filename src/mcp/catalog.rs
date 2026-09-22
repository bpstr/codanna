//! Typed tool names, argument vocabulary, positional sugar and authorization.
//! Handler schemas remain derived from request types. A completeness test compares
//! their generated router against this descriptor set so additions cannot disappear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Reindex,
}
macro_rules! tools {
    ($($kind:ident => ($name:literal,[$($key:literal),*],[$($required:literal),*],$positional:expr,$access:ident)),+ $(,)?) => {
        #[derive(Clone,Copy,Debug,PartialEq,Eq,Hash)]
        pub enum ToolKind { $($kind),+ }
        impl ToolKind {
            pub const ALL:&'static [Self]=&[$(Self::$kind),+];
            pub const fn name(self)->&'static str {match self {$(Self::$kind=>$name),+}}
            pub fn parse(name:&str)->Option<Self> {match name {$($name=>Some(Self::$kind)),+,_=>None}}
            pub const fn params(self)->(&'static [&'static str],&'static [&'static str]) {match self {$(Self::$kind=>(&[$($key),*],&[$($required),*])),+}}
            pub const fn positional(self)->Option<&'static str> {match self {$(Self::$kind=>$positional),+}}
            pub const fn access(self)->Access {match self {$(Self::$kind=>Access::$access),+}}
            pub fn names()->String {Self::ALL.iter().map(|kind|kind.name()).collect::<Vec<_>>().join(", ")}
        }
    }
}
tools! {
    FindSymbol => ("find_symbol",["name","symbol_id","lang","limit","offset"],["name"],Some("name"),Read),
    GetCalls => ("get_calls",["function_name","symbol_id"],["function_name","symbol_id"],Some("function_name"),Read),
    FindCallers => ("find_callers",["function_name","symbol_id"],["function_name","symbol_id"],Some("function_name"),Read),
    AnalyzeImpact => ("analyze_impact",["symbol_name","symbol_id","max_depth","depth"],["symbol_name","symbol_id"],Some("symbol_name"),Read),
    GetIndexInfo => ("get_index_info",[],[],None,Read),
    SearchSymbols => ("search_symbols",["query","limit","kind","module","lang","path_prefix"],["query"],Some("query"),Read),
    SemanticSearchDocs => ("semantic_search_docs",["query","limit","threshold","lang"],["query"],Some("query"),Read),
    SemanticSearchWithContext => ("semantic_search_with_context",["query","limit","threshold","lang"],["query"],Some("query"),Read),
    SearchDocuments => ("search_documents",["query","collection","limit"],["query"],Some("query"),Read),
    SearchTicketContext => ("search_ticket_context",["query","code_limit","document_limit","conversation_limit","collection","code_path_prefix","include_semantic_code","include_conversations"],["query"],Some("query"),Read),
    SearchContext => ("search_context",["query","code_limit","document_limit","conversation_limit","collection","code_path_prefix"],["query"],Some("query"),Read),
}
