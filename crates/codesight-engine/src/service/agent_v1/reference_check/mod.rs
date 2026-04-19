//! Reference existence / usage evidence for `check_references`.

mod evidence;

use serde_json::{Value, json};

use codesight_analysis::evidence::EvidenceNote;

use codesight_storage::storage::IndexStore;

use self::evidence::{IdentifierReferenceEvidence, ReferenceEvidenceProvider};

pub fn check_identifiers_batch(
    store: &IndexStore,
    identifiers: &[String],
    search_content: bool,
) -> anyhow::Result<Value> {
    let provider = ReferenceEvidenceProvider::from_store(store)?;
    let results: Vec<Value> = identifiers
        .iter()
        .map(|identifier| identifier_result(provider.collect(identifier, search_content)))
        .collect();

    Ok(json!({
        "checked_count": results.len(),
        "results": results,
    }))
}

pub fn check_single_identifier(
    store: &IndexStore,
    identifier: &str,
    search_content: bool,
) -> anyhow::Result<Value> {
    let provider = ReferenceEvidenceProvider::from_store(store)?;
    Ok(identifier_result(
        provider.collect(identifier, search_content),
    ))
}

pub fn check_refs_existence(
    store: &IndexStore,
    refs: &[(String, String)],
) -> anyhow::Result<Value> {
    // Two batch queries replace 2N single-key calls
    // (symbol_record + file_exists per ref).
    let symbol_ids: Vec<&str> = refs
        .iter()
        .filter(|(k, _)| k == "symbol")
        .map(|(_, id)| id.as_str())
        .collect();
    let file_ids: Vec<&str> = refs
        .iter()
        .filter(|(k, _)| k == "file")
        .map(|(_, id)| id.as_str())
        .collect();
    let symbol_hits = store.symbol_records_batch(&symbol_ids)?;
    let file_hits = store.files_exist_batch(&file_ids)?;

    let mut results: Vec<Value> = Vec::with_capacity(refs.len());
    for (kind, id) in refs {
        let exists = match kind.as_str() {
            "symbol" => symbol_hits.contains_key(id.as_str()),
            "file" => file_hits.contains(id.as_str()),
            _ => false,
        };
        results.push(json!({
            "ref": { "stable_id": id, "kind": kind },
            "exists": exists,
            "stale": false,
        }));
    }
    Ok(json!({
        "checked_count": results.len(),
        "results": results,
    }))
}

fn identifier_result(evidence: IdentifierReferenceEvidence) -> Value {
    json!({
        "identifier": evidence.identifier,
        "is_referenced": evidence.is_referenced(),
        "import_count": evidence.import_files.len(),
        "content_count": evidence.content_count(),
        "referencing_files": evidence.referencing_files(),
        "evidence_notes": serialize_notes(&evidence.evidence_notes),
        "evidence_breakdown": {
            "structural_import_files": evidence.import_files,
            "symbol_reference_files": evidence.symbol_reference_files,
            "fallback_content_files": evidence.fallback_content_files,
        },
    })
}

fn serialize_notes(notes: &[EvidenceNote]) -> Value {
    serde_json::to_value(notes).unwrap_or_else(|_| json!([]))
}
