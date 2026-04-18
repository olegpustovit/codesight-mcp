//! Analysis helpers used by `agent-v1` handlers (dead code, reference checks, quality).

pub mod agent_readiness;
pub mod complexity;
pub mod dead_code;
pub mod evidence;
pub mod graph;
pub mod impact;
pub mod quality;
pub mod quality_signal;
pub mod test_gaps;
pub mod thresholds;
