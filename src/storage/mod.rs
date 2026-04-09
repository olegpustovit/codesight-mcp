//! Root composition facade for storage domains.

pub mod doc_index;

pub use codesight_storage::storage::{
    AnalysisSymbol, IndexStore, RepoMetadata, RetrievedSymbol, SearchSymbol, SymbolMetadata,
    SymbolRecord, SymbolSourceRecord, list_repo_summaries,
};
pub use doc_index::{
    DocFileUpdate, DocIndexSummary, DocStore, RetrievedSection, SearchSection, SectionRecord,
    index_local_docs, list_doc_summaries,
};
