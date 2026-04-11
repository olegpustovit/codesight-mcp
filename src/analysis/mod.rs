//! Analysis helpers used by `agent-v1` handlers (dead code, reference checks, cross-repo map).

pub mod complexity;
pub mod cross_repo_map;
pub mod dead_code;
pub mod evidence;
pub mod graph;
pub mod impact;
pub mod quality;
pub mod reference_check;
