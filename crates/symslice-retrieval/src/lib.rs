//! Ranking, semantic retrieval, fuzzy matching, and doc section parsing.
//!
//! This domain sits between persistence (`storage`) and the MCP/service
//! boundary. Semantic retrieval runs on a bundled on-device embedder
//! (`embedder`) — there is no external LLM summarizer.

pub mod embedder;
pub mod fusion;
pub mod fuzzy;
pub mod sections;
