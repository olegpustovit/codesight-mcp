//! Detect whether extracted import edges reference an external package name.
//!
//! This replaces ad-hoc “import-looking line contains package substring” scans in handlers.
//! A remaining substring match on [`ImportEdge::specifier`] is intentional heuristic parity
//! with the previous line-based cross-repo detector (minimize false negatives).

use super::ImportEdge;

pub fn import_edges_reference_package(edges: &[ImportEdge], pkg_name: &str) -> bool {
    if pkg_name.is_empty() {
        return false;
    }
    for e in edges {
        if import_edge_specifier_hits_package(&e.specifier, pkg_name) {
            return true;
        }
    }
    false
}

fn import_edge_specifier_hits_package(specifier: &str, pkg_name: &str) -> bool {
    if specifier == pkg_name {
        return true;
    }
    if specifier.starts_with('@') && specifier.contains(pkg_name) {
        return true;
    }
    if specifier.starts_with(&format!("{pkg_name}/"))
        || specifier.starts_with(&format!("{pkg_name}."))
        || specifier.ends_with(&format!("/{pkg_name}"))
    {
        return true;
    }
    specifier.contains(pkg_name)
}
