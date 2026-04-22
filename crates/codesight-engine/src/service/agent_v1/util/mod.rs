//! Shared helpers for agent-v1 handlers, split by intent.

mod json_shape;
mod neighbors;
mod scope;
mod token;

pub(in crate::service) use json_shape::{
    doc_format_to_type_label, err_item, resolve_symbol_id, search_hit_section, search_hit_symbol,
};
pub(in crate::service) use neighbors::{
    direct_child_sections, heading_path_ancestors, outline_neighbors,
};
pub(in crate::service) use scope::path_matches_filter;
pub(in crate::service) use token::{
    pack_catalog_items_to_token_budget, token_estimate_for_context,
    truncate_content_to_token_budget,
};
