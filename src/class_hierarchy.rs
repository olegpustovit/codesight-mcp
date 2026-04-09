//! Root composition facade for class hierarchy analysis.

pub use codesight_storage::class_hierarchy::{
    ClassIndexRow, HierarchyNode, HierarchyView, compute_hierarchy_view, header_line_from_source,
    parse_bases,
};
