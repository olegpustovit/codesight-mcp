//! Security policy. Everything the engine enforces about *what it is allowed
//! to touch* lives here. Currently one concern: path trust (reject ingest of
//! broad roots like `/`, `/home`, unless the operator has whitelisted them).

mod trust;

pub use trust::{TrustCheckResult, check_trust, is_broad_path};
