//! Residual extractor facade.
//!
//! Category: `host_semantics`.
//! Exit criteria: delete this facade once Ansible play/task semantics can be expressed through
//! shared host-semantic providers over the YAML query pack.

pub use crate::extractors::residual::host_semantics::yaml::YamlExtractor;
