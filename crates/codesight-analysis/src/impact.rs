//! Impact payload builders.
//!
//! Two user-facing surfaces live here:
//!
//! - [`rename_safety_payload`] — symbol-level rename conflict check. Walks the
//!   defining file's outline plus importers for shape collisions.
//! - [`preview_path_rename`] — module-path prefix rewrite. Per-language logic
//!   is owned by `Language::path_rename_engine` in the language-support crate;
//!   this is a thin dispatch that also surfaces a consistent "unsupported
//!   language" error envelope for callers.
//!
//! The `PathRename*` shared types are re-exported from
//! [`codesight_language_support::path_rename`] so tool handlers can import
//! them from one place.

use std::collections::HashSet;

use serde_json::{Value, json};

pub use codesight_language_support::path_rename::{
    PathRenameChange, PathRenameCollision, PathRenameFilePlan, PathRenamePlan, PathRenameUnhandled,
    affected_paths, summarize as summarize_path_rename,
};
use codesight_storage::storage::IndexStore;

// -----------------------------------------------------------------------------
// rename_safety — symbol-level rename conflict check
// -----------------------------------------------------------------------------

pub fn rename_safety_payload(
    store: &IndexStore,
    symbol_id: &str,
    new_name: &str,
) -> anyhow::Result<Option<Value>> {
    let Some(record) = store.symbol_record(symbol_id)? else {
        return Ok(None);
    };

    let symbol_name = record
        .id
        .rsplit("::")
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("")
        .to_string();

    let file_path = &record.file_path;
    let outline = store.file_outline(file_path)?;
    let mut conflicts = Vec::new();

    for symbol in &outline {
        if symbol["name"].as_str() == Some(new_name) && symbol["id"].as_str() != Some(symbol_id) {
            conflicts.push(json!({
                "existing_name": new_name,
                "file": file_path,
                "kind": symbol["kind"],
                "id": symbol["id"],
            }));
        }
    }

    let adjacency = store.dependency_graph()?;
    let mut checked_files = HashSet::from([file_path.clone()]);
    let mut reverse_adj = std::collections::HashMap::<String, Vec<String>>::new();
    for (source, targets) in &adjacency {
        for target in targets {
            reverse_adj
                .entry(target.clone())
                .or_default()
                .push(source.clone());
        }
    }

    if let Some(importers) = reverse_adj.get(file_path) {
        // Batch-fetch all importer outlines in one query instead of N.
        let importer_refs: Vec<&str> = importers.iter().map(String::as_str).collect();
        let outlines = store.file_outlines_batch(&importer_refs)?;
        for importer in importers {
            checked_files.insert(importer.clone());
            if let Some(outline) = outlines.get(importer.as_str()) {
                for symbol in outline {
                    if symbol["name"].as_str() == Some(new_name) {
                        conflicts.push(json!({
                            "existing_name": new_name,
                            "file": importer,
                            "kind": symbol["kind"],
                            "id": symbol["id"],
                        }));
                    }
                }
            }
        }
    }

    // P0.3 — trait dispatch awareness. For Rust symbols whose defining file
    // declares the enclosing `trait <Owner> { … }`, a rename is only safe
    // when every `impl <Owner> for X { fn <name> … }` site is rewritten
    // alongside. Enumerate those sites so agents can't silently ship a
    // half-renamed trait.
    //
    // Qualified names like `Struct.method` carry the receiver — strip it so
    // the heuristic searches for the bare Rust identifier `method` in source.
    let symbol_leaf = symbol_name
        .rsplit('.')
        .next()
        .unwrap_or(symbol_name.as_str());
    let (trait_owner, required_edits) =
        detect_trait_impl_sites(store, file_path, symbol_leaf)?;
    let required_edits_rendered: Vec<Value> = required_edits
        .iter()
        .map(|file| {
            json!({
                "file": file,
                "reason": "trait_impl_method_must_be_renamed",
                "detail": "file contains `impl <Trait> for X` that must rename this method in lockstep",
            })
        })
        .collect();
    let trait_method_detected = trait_owner.is_some();
    let impl_count = required_edits.len();

    let safe = conflicts.is_empty() && !trait_method_detected;

    let mut payload = json!({
        "analysis": "rename_safety",
        "safe": safe,
        "conflicts": conflicts,
        "checked_files": checked_files.len(),
        "symbol": {
            "name": symbol_name,
            "id": symbol_id,
            "file": file_path,
        },
        "new_name": new_name,
        "trait_dispatch": {
            "detected_trait_owner": trait_owner,
            "required_edits": required_edits_rendered,
            "impl_site_count": impl_count,
        },
    });

    if trait_method_detected {
        payload["warnings"] = json!([{
            "code": "TRAIT_METHOD_RENAME",
            "message": format!(
                "'{symbol_name}' appears to be a trait method on '{}'. Every `impl <Trait> for X` site in `required_edits` must rename this method in the same patch.",
                trait_owner.as_deref().unwrap_or("(unknown trait)"),
            ),
        }]);
    }

    Ok(Some(payload))
}

/// Heuristic trait-method detection. Returns `(Some(trait_name), impl_files)`
/// when:
/// - `file_path` is Rust and declares a `trait <Trait> { fn <symbol_name>(…) }`
///   block that contains `symbol_name`.
/// - Any other indexed Rust file contains `impl <Trait> for X { fn <symbol_name>`.
///
/// Returns `(None, [])` for non-Rust files or when no trait declaration
/// contains the symbol. This intentionally over-triggers on Rust-only because
/// a false positive costs one extra confirmation while a false negative ships
/// a broken rename.
fn detect_trait_impl_sites(
    store: &IndexStore,
    file_path: &str,
    symbol_name: &str,
) -> anyhow::Result<(Option<String>, Vec<String>)> {
    if symbol_name.is_empty() {
        return Ok((None, Vec::new()));
    }
    if file_path.ends_with(".rs") {
        detect_rust_trait_impl_sites(store, file_path, symbol_name)
    } else if file_path.ends_with(".go") {
        // ROADMAP P0.3 #4: Go interface-method dispatch. A rename on an
        // interface method must cover every receiver with a matching
        // method name. We can't do full type inference, so we over-trigger:
        // any file with a `func (…) method(` is flagged as a candidate impl.
        detect_go_interface_impl_sites(store, file_path, symbol_name)
    } else {
        Ok((None, Vec::new()))
    }
}

fn detect_rust_trait_impl_sites(
    store: &IndexStore,
    file_path: &str,
    symbol_name: &str,
) -> anyhow::Result<(Option<String>, Vec<String>)> {
    let contents = store.all_file_contents().unwrap_or_default();
    let defining = contents
        .iter()
        .find(|(p, _)| p == file_path)
        .map(|(_, c)| c.as_str())
        .unwrap_or("");

    // Branch A — the starting file declares the trait that owns this method.
    // Branch B — the starting file is an `impl <Trait> for X` site; find the
    //            trait by scanning the repo for matching `trait <Trait>` with
    //            this method name, then enumerate every impl.
    let trait_owner = owning_trait_for_symbol(defining, symbol_name).or_else(|| {
        owning_trait_from_impl_site(defining, symbol_name).and_then(|candidate| {
            contents.iter().find_map(|(_, c)| {
                owning_trait_for_symbol(c, symbol_name)
                    .filter(|found| found == &candidate)
            })
        })
    });
    let Some(trait_owner) = trait_owner else {
        return Ok((None, Vec::new()));
    };
    let mut hits = Vec::new();
    for (p, content) in &contents {
        if p == file_path {
            continue;
        }
        if !p.ends_with(".rs") {
            continue;
        }
        if file_impls_trait_method(content, &trait_owner, symbol_name)
            || owning_trait_for_symbol(content, symbol_name).as_deref() == Some(trait_owner.as_str())
        {
            hits.push(p.clone());
        }
    }
    hits.sort();
    hits.dedup();
    Ok((Some(trait_owner), hits))
}

/// Scan a Go source for a `type <Name> interface { ... <method>(... }` block.
/// Returns the interface name when a matching method signature is found.
fn owning_interface_for_method_go(source: &str, method: &str) -> Option<String> {
    let mut current_iface: Option<String> = None;
    let mut depth: i32 = 0;
    let method_prefix = format!("{method}(");
    for raw in source.lines() {
        let line = raw.trim_start();
        if current_iface.is_none() {
            if let Some(rest) = line.strip_prefix("type ")
                && let Some(idx) = rest.find(" interface")
            {
                let name = rest[..idx].trim_end_matches(&[' ', '[', '\t'][..]);
                // Skip generic type-parameter brackets if present.
                let name = name.split(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap_or(name);
                if !name.is_empty() {
                    current_iface = Some(name.to_string());
                    depth = if line.contains('{') { 1 } else { 0 };
                }
            }
        } else {
            if line.starts_with('}') {
                depth -= 1;
                if depth <= 0 {
                    current_iface = None;
                    depth = 0;
                }
                continue;
            }
            if line.contains('{') {
                depth += 1;
            }
            if line.starts_with(&method_prefix)
                || line
                    .split_whitespace()
                    .next()
                    .map(|first| first == method || first.starts_with(&method_prefix))
                    .unwrap_or(false)
            {
                return current_iface.clone();
            }
        }
    }
    None
}

/// Best-effort scan for `func (recv) <method>(...)` method receivers in a Go
/// file. Over-triggers across unrelated interfaces — same name is treated as
/// a candidate implementor.
fn file_has_go_method_receiver(source: &str, method: &str) -> bool {
    let pat = format!(") {method}(");
    let pat_star = format!(") *{method}("); // unlikely but cheap to include
    source
        .lines()
        .any(|line| line.contains(pat.as_str()) || line.contains(pat_star.as_str()))
}

fn detect_go_interface_impl_sites(
    store: &IndexStore,
    file_path: &str,
    symbol_name: &str,
) -> anyhow::Result<(Option<String>, Vec<String>)> {
    let contents = store.all_file_contents().unwrap_or_default();
    let defining = contents
        .iter()
        .find(|(p, _)| p == file_path)
        .map(|(_, c)| c.as_str())
        .unwrap_or("");

    let iface = owning_interface_for_method_go(defining, symbol_name).or_else(|| {
        // If the defining file is an implementation, find an interface with
        // the method elsewhere in the repo.
        contents
            .iter()
            .find_map(|(_, c)| owning_interface_for_method_go(c, symbol_name))
    });
    let Some(iface) = iface else {
        return Ok((None, Vec::new()));
    };
    let mut hits = Vec::new();
    for (p, content) in &contents {
        if p == file_path {
            continue;
        }
        if !p.ends_with(".go") {
            continue;
        }
        if file_has_go_method_receiver(content, symbol_name) {
            hits.push(p.clone());
        }
    }
    hits.sort();
    hits.dedup();
    Ok((Some(iface), hits))
}

/// Inspect an `impl <Trait> for X` site and return the trait name when the
/// block contains a method named `symbol_name`. Mirror of
/// [`file_impls_trait_method`] that returns the trait name instead of a bool.
fn owning_trait_from_impl_site(source: &str, symbol_name: &str) -> Option<String> {
    let fn_needle = format!("fn {symbol_name}");
    let mut current_trait: Option<String> = None;
    let mut depth: i32 = 0;
    let mut in_impl = false;
    for raw in source.lines() {
        let line = raw.trim_start();
        if !in_impl {
            if let Some(name) = parse_impl_trait_header(line) {
                current_trait = Some(name);
                in_impl = true;
                depth = line.matches('{').count() as i32 - line.matches('}').count() as i32;
                if depth <= 0 {
                    in_impl = false;
                    current_trait = None;
                }
                continue;
            }
        } else {
            depth += line.matches('{').count() as i32;
            depth -= line.matches('}').count() as i32;
            if line.contains(&fn_needle)
                && line
                    .split(&fn_needle)
                    .nth(1)
                    .is_some_and(|tail| matches!(tail.chars().next(), Some('(') | Some('<')))
            {
                return current_trait;
            }
            if depth <= 0 {
                in_impl = false;
                current_trait = None;
            }
        }
    }
    None
}

/// Parse a Rust `impl [<generics>] <Trait> for <Type>` line and return the
/// trait name. Returns `None` for inherent impls (`impl Foo { ... }`) and for
/// anything that doesn't start the `impl` keyword.
fn parse_impl_trait_header(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("unsafe ")
        .unwrap_or(line)
        .strip_prefix("impl")?
        .trim_start();
    // Skip optional generic block `<...>`.
    let rest = if let Some(r) = rest.strip_prefix('<') {
        let mut depth = 1i32;
        let mut idx = 0usize;
        for (i, c) in r.char_indices() {
            match c {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        idx = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        r[idx..].trim_start()
    } else {
        rest
    };
    // Find " for " separator — without it this is an inherent impl.
    let for_pos = rest.find(" for ")?;
    let trait_part = rest[..for_pos].trim();
    // Strip any trailing generic args on the trait itself.
    let name: String = trait_part
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

/// Walk `source` line-by-line looking for a `trait <Name> {` block whose body
/// contains a function declaration named `symbol_name`. Returns `Some(name)` on
/// first match. Tracks brace depth so nested blocks don't fool us.
fn owning_trait_for_symbol(source: &str, symbol_name: &str) -> Option<String> {
    let needle = format!("fn {symbol_name}");
    let mut in_trait: Option<String> = None;
    let mut depth: i32 = 0;
    for raw in source.lines() {
        let line = raw.trim_start();
        if in_trait.is_none() {
            if let Some(name) = parse_trait_header(line) {
                in_trait = Some(name);
                depth = line.matches('{').count() as i32 - line.matches('}').count() as i32;
                if depth <= 0 {
                    in_trait = None;
                }
                continue;
            }
        } else {
            depth += line.matches('{').count() as i32;
            depth -= line.matches('}').count() as i32;
            if line.contains(&needle)
                && line
                    .split(&needle)
                    .nth(1)
                    .is_some_and(|tail| matches!(tail.chars().next(), Some('(') | Some('<')))
            {
                return in_trait;
            }
            if depth <= 0 {
                in_trait = None;
            }
        }
    }
    None
}

fn parse_trait_header(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("pub ")
        .or_else(|| line.strip_prefix("pub(crate) "))
        .or_else(|| line.strip_prefix("pub(super) "))
        .or_else(|| line.strip_prefix("unsafe "))
        .unwrap_or(line);
    let rest = rest.strip_prefix("trait ")?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

fn file_impls_trait_method(source: &str, trait_name: &str, symbol_name: &str) -> bool {
    let impl_prefix_a = format!("impl {trait_name} for ");
    let impl_prefix_b = format!("impl<");
    let fn_needle = format!("fn {symbol_name}");
    let mut in_impl = false;
    let mut depth: i32 = 0;
    for raw in source.lines() {
        let line = raw.trim_start();
        if !in_impl {
            if line.starts_with(&impl_prefix_a)
                || (line.starts_with(&impl_prefix_b)
                    && line.contains(&format!(" {trait_name} for ")))
            {
                in_impl = true;
                depth = line.matches('{').count() as i32 - line.matches('}').count() as i32;
                if depth <= 0 {
                    in_impl = false;
                }
                continue;
            }
        } else {
            depth += line.matches('{').count() as i32;
            depth -= line.matches('}').count() as i32;
            if line.contains(&fn_needle)
                && line
                    .split(&fn_needle)
                    .nth(1)
                    .is_some_and(|tail| matches!(tail.chars().next(), Some('(') | Some('<')))
            {
                return true;
            }
            if depth <= 0 {
                in_impl = false;
            }
        }
    }
    false
}

// -----------------------------------------------------------------------------
// path_rename — dispatch to the language-owned engine
// -----------------------------------------------------------------------------

/// Preview a module-path rewrite using the language associated with `lang_id`
/// (e.g. `"rust"`). Returns `Ok(None)` when the language doesn't expose a
/// path-rename engine so callers can render a consistent error envelope.
pub fn preview_path_rename(
    store: &IndexStore,
    lang_id: &str,
    from: &str,
    to: &str,
    scope: Option<&str>,
    partial_mode: &str,
) -> anyhow::Result<Option<PathRenamePlan>> {
    let Some(lang) = codesight_language_support::builtin_registry().for_language_name(lang_id)
    else {
        return Ok(None);
    };
    let Some(engine) = lang.path_rename_engine() else {
        return Ok(None);
    };
    let root = store.root_path()?;
    engine
        .preview(&root, from, to, scope, partial_mode)
        .map(Some)
}
