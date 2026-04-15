//! Index-time concerns: structural facts extracted from source (imports, etc.).
//! This module must not depend on public MCP/tool response shaping.

pub mod code_index;
pub mod dep_graph;
pub mod imports;
pub mod packages;
