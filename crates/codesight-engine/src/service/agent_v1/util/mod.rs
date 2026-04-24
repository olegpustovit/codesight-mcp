//! Shared helpers for agent-v1 handlers, split by intent.

pub(in crate::service) mod graph;
mod json_shape;
mod neighbors;
mod scope;
mod token;

#[allow(unused_imports)]
pub(in crate::service) use json_shape::{
    RefResolution, doc_format_to_type_label, err_item, ref_requires_strict_resolution,
    resolve_symbol_id, resolve_symbol_id_strict, search_hit_section, search_hit_symbol,
};
pub(in crate::service) use neighbors::{
    direct_child_sections, heading_path_ancestors, outline_neighbors,
};
pub(in crate::service) use scope::path_matches_filter;
pub(in crate::service) use token::{
    pack_catalog_items_to_token_budget, token_estimate_for_context,
    truncate_content_to_token_budget,
};
