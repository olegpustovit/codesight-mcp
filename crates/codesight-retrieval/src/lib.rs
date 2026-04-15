//! Ranking, semantic retrieval, fuzzy matching, summarization, and doc section parsing.
//!
//! This domain sits between persistence (`storage`) and the MCP/service boundary.

pub mod bm25_reference;
pub mod doc_summarizer;
pub mod embedder;
pub mod fusion;
pub mod fuzzy;
pub mod sections;
pub mod summarizer;
