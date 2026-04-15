//! Detect whether extracted import edges reference an external package name.
//!
//! Matching is rooted in language-aware package parsing, not raw substring scans.

use serde::Serialize;

use crate::packages::extract_root_package_from_specifier;

use super::ImportEdge;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageMatchKind {
    ExactSpecifier,
    RootSpecifier,
    NormalizedAlias,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageReferenceEvidence {
    pub package_name: String,
    pub specifier: String,
    pub language: String,
    pub match_kind: PackageMatchKind,
}

#[derive(Debug, Clone)]
pub struct PackageReferenceMatcher {
    package_name: String,
    exact_candidates: Vec<String>,
    normalized_candidates: Vec<String>,
}

impl PackageReferenceMatcher {
    pub fn new(package_name: &str) -> Self {
        let mut normalized_candidates = Vec::new();
        let normalized = normalize_package_name(package_name);
        if !normalized.is_empty() {
            normalized_candidates.push(normalized);
        }

        Self {
            package_name: package_name.to_string(),
            exact_candidates: vec![package_name.to_string()],
            normalized_candidates,
        }
    }

    pub fn match_edges(
        &self,
        edges: &[ImportEdge],
        import_language: &str,
    ) -> Option<PackageReferenceEvidence> {
        edges
            .iter()
            .find_map(|edge| self.match_edge(edge, import_language))
    }

    fn match_edge(
        &self,
        edge: &ImportEdge,
        import_language: &str,
    ) -> Option<PackageReferenceEvidence> {
        let specifier = edge.specifier.trim();
        if specifier.is_empty() {
            return None;
        }

        let root_specifier = extract_root_package_from_specifier(specifier, import_language);
        if self
            .exact_candidates
            .iter()
            .any(|candidate| candidate == specifier)
        {
            return Some(self.evidence(
                specifier,
                import_language,
                PackageMatchKind::ExactSpecifier,
            ));
        }

        if !root_specifier.is_empty()
            && self
                .exact_candidates
                .iter()
                .any(|candidate| candidate == &root_specifier)
        {
            return Some(self.evidence(
                specifier,
                import_language,
                PackageMatchKind::RootSpecifier,
            ));
        }

        let normalized_root = normalize_package_name(&root_specifier);
        if !normalized_root.is_empty()
            && self
                .normalized_candidates
                .iter()
                .any(|candidate| candidate == &normalized_root)
        {
            return Some(self.evidence(
                specifier,
                import_language,
                PackageMatchKind::NormalizedAlias,
            ));
        }

        None
    }

    fn evidence(
        &self,
        specifier: &str,
        import_language: &str,
        match_kind: PackageMatchKind,
    ) -> PackageReferenceEvidence {
        PackageReferenceEvidence {
            package_name: self.package_name.clone(),
            specifier: specifier.to_string(),
            language: import_language.to_string(),
            match_kind,
        }
    }
}

pub fn package_reference_evidence(
    edges: &[ImportEdge],
    import_language: &str,
    package_name: &str,
) -> Option<PackageReferenceEvidence> {
    PackageReferenceMatcher::new(package_name).match_edges(edges, import_language)
}

fn normalize_package_name(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace('_', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_python_normalized_package_alias() {
        let edges = vec![ImportEdge {
            specifier: "my_lib.utils".to_string(),
            names: vec!["helper".to_string()],
        }];
        let evidence = package_reference_evidence(&edges, "python", "my-lib").expect("match");
        assert_eq!(evidence.match_kind, PackageMatchKind::NormalizedAlias);
    }

    #[test]
    fn matches_scoped_js_package_by_root() {
        let edges = vec![ImportEdge {
            specifier: "@org/toolkit/utils".to_string(),
            names: vec![],
        }];
        let evidence =
            package_reference_evidence(&edges, "javascript", "@org/toolkit").expect("match");
        assert_eq!(evidence.match_kind, PackageMatchKind::RootSpecifier);
    }

    #[test]
    fn does_not_match_unrelated_substrings() {
        let edges = vec![ImportEdge {
            specifier: "my_library".to_string(),
            names: vec![],
        }];
        assert!(package_reference_evidence(&edges, "python", "lib").is_none());
    }
}
