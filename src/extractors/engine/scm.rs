//! SCM query-based extractor engine.
//!
//! Each language is defined by a single `.scm` file containing tree-sitter queries.
//! The engine compiles the query, runs it via `QueryCursor`, and builds `StoredSymbol`
//! records from captures using containment-based scope inference.
//!
//! Language knowledge for symbol capture lives in `.scm` files. A small post-pass
//! for C/C++ headers lives in [`super::cpp_header_language_fallback`].

use anyhow::{Result, anyhow};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use crate::model::{StoredSymbol, SymbolKind};

use crate::extractors::LanguageExtractor;
use crate::extractors::common::{first_line, node_text, qualify, source_slice};

// ---------------------------------------------------------------------------
// Metadata parsed from header comments in .scm files
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ScmMetadata {
    pub name: String,
    pub extensions: Vec<String>,
    pub grammar: String,
    pub override_extractor: Option<String>,
}

/// Parse `; key: value` header lines from the top of a .scm file.
pub fn parse_scm_header(source: &str) -> ScmMetadata {
    let mut meta = ScmMetadata::default();
    for line in source.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with(';') {
            if !trimmed.is_empty() {
                break; // end of header
            }
            continue;
        }
        let comment = trimmed.trim_start_matches(';').trim();
        if let Some((key, value)) = comment.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "language" => meta.name = value.to_string(),
                "extensions" => {
                    meta.extensions = value.split_whitespace().map(String::from).collect()
                }
                "grammar" => meta.grammar = value.to_string(),
                "override" => meta.override_extractor = Some(value.to_string()),
                _ => {}
            }
        }
    }
    meta
}

// ---------------------------------------------------------------------------
// Per-pattern metadata from #set! directives
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct PatternProps {
    /// Skip this symbol if contained within a definition of this kind.
    exclude_inside: Option<String>,
    /// Create a scope entry but don't emit a StoredSymbol.
    scope_only: bool,
    /// Use this literal string as the name (e.g. "init" for Swift).
    fixed_name: Option<String>,
    /// Prepend this string to the captured @name text (e.g. "@keyframes ").
    name_prefix: Option<String>,
}

fn read_pattern_props(query: &Query, pattern_index: usize) -> PatternProps {
    let mut props = PatternProps::default();
    for p in query.property_settings(pattern_index) {
        match p.key.as_ref() {
            "exclude_inside" => {
                props.exclude_inside = p.value.as_ref().map(|v| v.to_string());
            }
            "scope_only" => props.scope_only = true,
            "fixed_name" => {
                props.fixed_name = p.value.as_ref().map(|v| v.to_string());
            }
            "name_prefix" => {
                props.name_prefix = p.value.as_ref().map(|v| v.to_string());
            }
            _ => {}
        }
    }
    props
}

// ---------------------------------------------------------------------------
// Raw captured symbol (before dedup + scope inference)
// ---------------------------------------------------------------------------

struct OwnedCapture {
    kind: SymbolKind,
    kind_str: String,
    name: String,
    name_start_byte: usize,
    def_start: usize,
    def_end: usize,
    def_start_row: usize,
    def_end_row: usize,
    def_signature: String,
    def_source_text: String,
    doc: Option<String>,
    scope_label: Option<String>,
    qualified_name_override: Option<String>,
    pattern_index: usize,
    props: PatternProps,
}

fn parse_symbol_kind(kind_str: &str) -> Option<SymbolKind> {
    Some(match kind_str {
        "function" => SymbolKind::Function,
        "class" => SymbolKind::Class,
        "method" => SymbolKind::Method,
        "constant" => SymbolKind::Constant,
        "type" => SymbolKind::Type,
        "module" | "namespace" => SymbolKind::Type,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// ScmExtractor
// ---------------------------------------------------------------------------

pub struct ScmExtractor {
    language_name: &'static str,
    extensions: &'static [&'static str],
    grammar: tree_sitter::Language,
    query: Query,
}

impl ScmExtractor {
    pub fn new(meta: &ScmMetadata, query_source: &str) -> Result<Self> {
        let grammar = crate::extractors::engine::builtin_grammar(&meta.grammar)
            .ok_or_else(|| anyhow!("unknown grammar: {}", meta.grammar))?;

        let ts_lang: tree_sitter::Language = grammar.clone().into();
        let query = Query::new(&ts_lang, query_source)
            .map_err(|e| anyhow!("query compile error for {}: {e}", meta.name))?;

        let language_name: &'static str = Box::leak(meta.name.clone().into_boxed_str());
        let exts: Vec<&'static str> = meta
            .extensions
            .iter()
            .map(|s| &*Box::leak(s.clone().into_boxed_str()))
            .collect();
        let extensions: &'static [&'static str] = Box::leak(exts.into_boxed_slice());

        Ok(Self {
            language_name,
            extensions,
            grammar,
            query,
        })
    }
}

impl LanguageExtractor for ScmExtractor {
    fn language_name(&self) -> &'static str {
        self.language_name
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        self.extensions
    }

    fn extract(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&self.grammar)
            .map_err(|e| anyhow!("set_language: {e}"))?;
        let tree = parser
            .parse(source, None)
            .ok_or_else(|| anyhow!("parse returned no tree"))?;
        let mut cursor = QueryCursor::new();

        let capture_names = self.query.capture_names();

        // --- Phase 1: Collect raw captures ---
        // QueryMatches is a StreamingIterator — text predicates (#match?, #eq?) are
        // checked automatically. We extract owned data from each match before advancing.
        let mut raws: Vec<OwnedCapture> = Vec::new();
        let mut standalone_docs: Vec<(usize, String)> = Vec::new(); // (byte_offset, cleaned_doc)

        let mut matches = cursor.matches(&self.query, tree.root_node(), source.as_bytes());
        while let Some(m) = matches.next() {
            let mut props = read_pattern_props(&self.query, m.pattern_index);

            let mut def_start = 0usize;
            let mut def_end = 0usize;
            let mut def_start_row = 0usize;
            let mut def_end_row = 0usize;
            let mut def_kind_str: Option<String> = None;
            let mut def_signature = String::new();
            let mut def_source_text = String::new();
            let mut name_text: Option<String> = None;
            let mut name_start_byte = 0usize;
            let mut doc_text: Option<String> = None;
            let mut scope_text: Option<String> = None;
            let mut qname_text: Option<String> = None;
            let mut has_def = false;

            let mut capture_props = PatternProps::default();

            for cap in m.captures {
                let cap_name = capture_names[cap.index as usize];
                if let Some(rest) = cap_name.strip_prefix("definition.") {
                    has_def = true;
                    def_start = cap.node.start_byte();
                    def_end = cap.node.end_byte();
                    def_start_row = cap.node.start_position().row;
                    def_end_row = cap.node.end_position().row;
                    // Parse suffixes: "constant.nofunc" → kind="constant", exclude_inside="function"
                    // "namespace.scope_only" → kind="namespace", scope_only=true
                    let mut kind_part = rest;
                    if let Some(base) = rest.strip_suffix(".nofunc") {
                        kind_part = base;
                        capture_props.exclude_inside = Some("function".to_string());
                    } else if let Some(base) = rest.strip_suffix(".scope_only") {
                        kind_part = base;
                        capture_props.scope_only = true;
                    }
                    def_kind_str = Some(kind_part.to_string());
                    def_signature = first_line(cap.node, source).unwrap_or_default();
                    def_source_text = source_slice(cap.node, source).unwrap_or("").to_string();
                } else {
                    match cap_name {
                        "name" => {
                            name_text = node_text(cap.node, source).ok().map(|s| s.to_string());
                            name_start_byte = cap.node.start_byte();
                        }
                        "doc" => {
                            doc_text = node_text(cap.node, source).ok().map(|s| clean_docstring(s));
                        }
                        "scope" => {
                            scope_text = node_text(cap.node, source)
                                .ok()
                                .map(|t| t.trim_start_matches('*').to_string());
                        }
                        "qualified_name" => {
                            qname_text = node_text(cap.node, source).ok().map(|s| s.to_string());
                        }
                        _ => {} // internal captures (_key, etc.)
                    }
                }
            }

            // Standalone @doc captures (no @definition.* in same match):
            // collect with the byte position from the match for containment lookup.
            if !has_def {
                if let Some(doc) = doc_text {
                    // Use the first capture's start byte as location hint.
                    let byte_pos = m.captures.first().map_or(0, |c| c.node.start_byte());
                    standalone_docs.push((byte_pos, doc));
                }
                continue;
            }
            let Some(kind_str) = def_kind_str else {
                continue;
            };

            // Resolve name: fixed_name > @name capture (with optional prefix)
            let name = if let Some(fixed) = &props.fixed_name {
                fixed.clone()
            } else if let Some(n) = name_text {
                if let Some(ref prefix) = props.name_prefix {
                    format!("{prefix}{n}")
                } else {
                    n
                }
            } else {
                continue;
            };

            let Some(kind) = parse_symbol_kind(&kind_str) else {
                continue;
            };

            // Merge capture-name props with #set! props (capture-name takes precedence).
            if capture_props.exclude_inside.is_some() {
                props.exclude_inside = capture_props.exclude_inside;
            }
            if capture_props.scope_only {
                props.scope_only = true;
            }
            if capture_props.fixed_name.is_some() {
                props.fixed_name = capture_props.fixed_name;
            }

            raws.push(OwnedCapture {
                kind,
                kind_str,
                name,
                name_start_byte,
                def_start,
                def_end,
                def_start_row,
                def_end_row,
                def_signature,
                def_source_text,
                doc: doc_text,
                scope_label: scope_text,
                qualified_name_override: qname_text,
                pattern_index: m.pattern_index,
                props,
            });
        }

        // --- Phase 2: Dedup by name position (lowest pattern_index wins) ---
        raws.sort_by_key(|r| (r.name_start_byte, r.pattern_index));
        raws.dedup_by(|b, a| a.name_start_byte == b.name_start_byte && a.name == b.name);

        // --- Phase 2b: Associate standalone @doc captures with symbols ---
        // A standalone doc at byte offset N belongs to the symbol whose span contains N
        // (tightest containing symbol). If no containing symbol is found, associate
        // with the *next* symbol after the doc (preceding-doc-comment pattern, e.g.
        // Lua `---` comments that sit above a function).
        if !standalone_docs.is_empty() {
            for (doc_byte, doc_text) in &standalone_docs {
                // First try: tightest containing symbol.
                let best = raws
                    .iter_mut()
                    .filter(|r| {
                        r.def_start <= *doc_byte && *doc_byte < r.def_end && r.doc.is_none()
                    })
                    .min_by_key(|r| r.def_end - r.def_start);
                if let Some(sym) = best {
                    sym.doc = Some(doc_text.clone());
                    continue;
                }
                // Fallback: next symbol whose def_start is after (or at) doc_byte.
                let next = raws
                    .iter_mut()
                    .filter(|r| r.def_start >= *doc_byte && r.doc.is_none())
                    .min_by_key(|r| r.def_start);
                if let Some(sym) = next {
                    sym.doc = Some(doc_text.clone());
                }
            }
        }

        // --- Phase 3: Sort by span for containment scoping ---
        raws.sort_by_key(|r| (r.def_start, std::cmp::Reverse(r.def_end)));

        // --- Phase 4: Containment-based scope inference + build StoredSymbols ---
        struct ScopeEntry {
            span_end: usize,
            name: String,
            kind_str: String,
        }
        let mut scope_stack: Vec<ScopeEntry> = Vec::new();
        let mut symbols = Vec::new();

        for raw in &raws {
            // Pop scopes that don't contain this symbol.
            while let Some(top) = scope_stack.last() {
                if top.span_end <= raw.def_start {
                    scope_stack.pop();
                } else {
                    break;
                }
            }

            // Check exclude_inside.
            if let Some(exclude) = &raw.props.exclude_inside {
                if scope_stack.iter().any(|s| s.kind_str == *exclude) {
                    scope_stack.push(ScopeEntry {
                        span_end: raw.def_end,
                        name: raw.name.clone(),
                        kind_str: raw.kind_str.clone(),
                    });
                    continue;
                }
            }

            // Build qualified name.
            let qualified_name = if let Some(qn) = &raw.qualified_name_override {
                qn.clone()
            } else {
                let scope_names: Vec<String> = scope_stack
                    .iter()
                    .map(|s| s.name.clone())
                    .chain(raw.scope_label.iter().cloned())
                    .collect();
                qualify(&scope_names, &raw.name)
            };

            // Push as scope for containment.
            scope_stack.push(ScopeEntry {
                span_end: raw.def_end,
                name: raw.name.clone(),
                kind_str: raw.kind_str.clone(),
            });

            if raw.props.scope_only {
                continue;
            }

            let summary = raw.doc.clone().unwrap_or_else(|| raw.def_signature.clone());
            let source_hash = blake3::hash(raw.def_source_text.as_bytes())
                .to_hex()
                .to_string();

            symbols.push(StoredSymbol {
                id: format!("{}::{}#{}", file_path, qualified_name, raw.kind.as_str()),
                language: self.language_name.to_string(),
                file_path: file_path.to_string(),
                qualified_name,
                name: raw.name.clone(),
                kind: raw.kind,
                signature: raw.def_signature.clone(),
                summary,
                start_line: (raw.def_start_row + 1) as u32,
                end_line: (raw.def_end_row + 1) as u32,
                start_byte: raw.def_start,
                end_byte: raw.def_end,
                source_hash,
                source_text: raw.def_source_text.clone(),
                file_size_bytes: source.len() as u64,
            });
        }

        super::cpp_header_language_fallback::adjust_language_for_cpp_header_if_plain_c(
            &mut symbols,
            self.language_name,
            file_path,
            source,
        );

        symbols.sort_by_key(|s| (s.start_byte, s.end_byte));
        Ok(symbols)
    }
}

// ---------------------------------------------------------------------------
// Docstring cleaning (universal, not language-specific)
// ---------------------------------------------------------------------------

fn clean_docstring(raw: &str) -> String {
    let trimmed = raw.trim();
    // Strip string delimiters (triple quotes, single quotes, etc.)
    let stripped = trimmed
        .strip_prefix("\"\"\"")
        .and_then(|s| s.strip_suffix("\"\"\""))
        .or_else(|| {
            trimmed
                .strip_prefix("'''")
                .and_then(|s| s.strip_suffix("'''"))
        })
        .or_else(|| trimmed.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
        .or_else(|| {
            trimmed
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })
        .unwrap_or(trimmed);

    // Also strip leading string prefix chars (r, u, f, b)
    let stripped = stripped.trim_start_matches(['r', 'R', 'u', 'U', 'f', 'F', 'b', 'B']);

    stripped
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Load all .scm extractors from embedded query files
// ---------------------------------------------------------------------------

/// Built-in query files, embedded at compile time.
pub const QUERY_FILES: &[(&str, &str)] = &[
    (
        "python",
        include_str!("../../../runtime/queries/python/symbols.scm"),
    ),
    (
        "bash",
        include_str!("../../../runtime/queries/bash/symbols.scm"),
    ),
    (
        "java",
        include_str!("../../../runtime/queries/java/symbols.scm"),
    ),
    (
        "go",
        include_str!("../../../runtime/queries/go/symbols.scm"),
    ),
    (
        "php",
        include_str!("../../../runtime/queries/php/symbols.scm"),
    ),
    (
        "javascript",
        include_str!("../../../runtime/queries/javascript/symbols.scm"),
    ),
    (
        "typescript",
        include_str!("../../../runtime/queries/typescript/symbols.scm"),
    ),
    (
        "tsx",
        include_str!("../../../runtime/queries/tsx/symbols.scm"),
    ),
    (
        "json",
        include_str!("../../../runtime/queries/json/symbols.scm"),
    ),
    (
        "css",
        include_str!("../../../runtime/queries/css/symbols.scm"),
    ),
    (
        "ruby",
        include_str!("../../../runtime/queries/ruby/symbols.scm"),
    ),
    (
        "lua",
        include_str!("../../../runtime/queries/lua/symbols.scm"),
    ),
    (
        "swift",
        include_str!("../../../runtime/queries/swift/symbols.scm"),
    ),
    (
        "rust_lang",
        include_str!("../../../runtime/queries/rust/symbols.scm"),
    ),
    (
        "cpp",
        include_str!("../../../runtime/queries/cpp/symbols.scm"),
    ),
    (
        "csharp",
        include_str!("../../../runtime/queries/csharp/symbols.scm"),
    ),
    (
        "kotlin",
        include_str!("../../../runtime/queries/kotlin/symbols.scm"),
    ),
    (
        "perl",
        include_str!("../../../runtime/queries/perl/symbols.scm"),
    ),
    (
        "elixir",
        include_str!("../../../runtime/queries/elixir/symbols.scm"),
    ),
    (
        "erlang",
        include_str!("../../../runtime/queries/erlang/symbols.scm"),
    ),
    (
        "dart",
        include_str!("../../../runtime/queries/dart/symbols.scm"),
    ),
    (
        "xml",
        include_str!("../../../runtime/queries/xml/symbols.scm"),
    ),
    (
        "nix",
        include_str!("../../../runtime/queries/nix/symbols.scm"),
    ),
    (
        "fortran",
        include_str!("../../../runtime/queries/fortran/symbols.scm"),
    ),
    (
        "gleam",
        include_str!("../../../runtime/queries/gleam/symbols.scm"),
    ),
];

pub fn build_scm_extractors() -> Result<Vec<Box<dyn LanguageExtractor>>> {
    let mut extractors: Vec<Box<dyn LanguageExtractor>> = Vec::new();

    for (_key, source) in QUERY_FILES {
        let meta = parse_scm_header(source);
        if meta.override_extractor.is_some() {
            continue; // handled by override registry
        }
        if meta.grammar.is_empty() {
            continue;
        }
        match ScmExtractor::new(&meta, source) {
            Ok(ext) => extractors.push(Box::new(ext)),
            Err(e) => {
                eprintln!("warning: failed to compile query for {}: {e}", meta.name);
            }
        }
    }

    Ok(extractors)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_extractor(key: &str) -> ScmExtractor {
        let source = QUERY_FILES
            .iter()
            .find(|(k, _)| *k == key)
            .unwrap_or_else(|| panic!("no query file for {key}"))
            .1;
        let meta = parse_scm_header(source);
        ScmExtractor::new(&meta, source).unwrap()
    }

    #[test]
    fn scm_bash_extracts_functions_and_constants() {
        let ext = make_extractor("bash");
        let src = r#"#!/bin/bash
MAX_RETRIES=3
TIMEOUT=30

greet() {
    echo "Hello $1"
}

function deploy {
    echo "deploying..."
}
"#;
        let symbols = ext.extract("deploy.sh", src).expect("bash");
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"greet"), "missing greet: {names:?}");
        assert!(names.contains(&"deploy"), "missing deploy: {names:?}");
        assert!(
            names.contains(&"MAX_RETRIES"),
            "missing MAX_RETRIES: {names:?}"
        );
    }

    #[test]
    fn scm_java_classes_methods_constants() {
        let ext = make_extractor("java");
        let src = r#"
public class UserService {
    private static final int MAX_RETRIES = 3;

    public void createUser(String name) {}
    public String getUser(int id) { return null; }
}

public interface Repo { void save(); }
public enum Color { RED, GREEN; }
"#;
        let symbols = ext.extract("Test.java", src).expect("java");
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"UserService"), "missing UserService");
        assert!(names.contains(&"createUser"), "missing createUser");
        assert!(names.contains(&"getUser"), "missing getUser");
        assert!(names.contains(&"Repo"), "missing Repo");
        assert!(names.contains(&"Color"), "missing Color");

        // Check containment-based scoping
        let method = symbols.iter().find(|s| s.name == "createUser").unwrap();
        assert_eq!(method.kind, SymbolKind::Method);
        assert_eq!(method.qualified_name, "UserService.createUser");
    }

    #[test]
    fn scm_python_decorators_docstrings_constants() {
        let ext = make_extractor("python");
        let src = r#"
MAX_RETRIES = 3

class UserService:
    """Handles user operations."""

    @classmethod
    def find_user(cls, user_id: str):
        """Find a user by id."""
        return user_id

def top_level(value):
    return value
"#;
        let symbols = ext.extract("svc.py", src).expect("python");

        // Check symbols exist
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"MAX_RETRIES"), "missing MAX_RETRIES");
        assert!(names.contains(&"UserService"), "missing UserService");
        assert!(names.contains(&"find_user"), "missing find_user");
        assert!(names.contains(&"top_level"), "missing top_level");

        // Method should be qualified and have decorator in source
        let method = symbols.iter().find(|s| s.name == "find_user").unwrap();
        assert_eq!(method.kind, SymbolKind::Method);
        assert_eq!(method.qualified_name, "UserService.find_user");
        assert!(
            method.source_text.contains("@classmethod"),
            "decorator not in source_text"
        );
    }

    #[test]
    fn scm_python_constants_excluded_inside_functions() {
        let ext = make_extractor("python");
        let src = r#"
TOP_LEVEL = 1

def func():
    INSIDE_FUNC = 2

class Cls:
    CLASS_CONST = 3
"#;
        let symbols = ext.extract("test.py", src).expect("python");
        let const_names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Constant)
            .map(|s| s.name.as_str())
            .collect();
        assert!(const_names.contains(&"TOP_LEVEL"), "missing TOP_LEVEL");
        assert!(const_names.contains(&"CLASS_CONST"), "missing CLASS_CONST");
        assert!(
            !const_names.contains(&"INSIDE_FUNC"),
            "INSIDE_FUNC should be excluded"
        );
    }

    #[test]
    fn scm_go_receiver_scoping() {
        let ext = make_extractor("go");
        let src = r#"
package p

const Version = "1"

type Server struct { addr string }

func (s *Server) Addr() string { return s.addr }
func main() {}
"#;
        let symbols = ext.extract("main.go", src).expect("go");
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Version" && s.kind == SymbolKind::Constant)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Server" && s.kind == SymbolKind::Class)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "main" && s.kind == SymbolKind::Function)
        );

        let method = symbols.iter().find(|s| s.name == "Addr").unwrap();
        assert_eq!(method.kind, SymbolKind::Method);
        assert_eq!(method.qualified_name, "Server.Addr");
    }

    #[test]
    fn scm_bash_query_compiles() {
        make_extractor("bash");
    }

    #[test]
    fn scm_java_query_compiles() {
        make_extractor("java");
    }

    #[test]
    fn scm_python_query_compiles() {
        make_extractor("python");
    }

    #[test]
    fn scm_go_query_compiles() {
        make_extractor("go");
    }

    #[test]
    fn scm_php_query_compiles() {
        make_extractor("php");
    }

    #[test]
    fn scm_javascript_query_compiles() {
        make_extractor("javascript");
    }

    #[test]
    fn scm_rust_query_compiles() {
        make_extractor("rust_lang");
    }

    #[test]
    fn scm_cpp_query_compiles() {
        make_extractor("cpp");
    }

    #[test]
    fn scm_csharp_query_compiles() {
        make_extractor("csharp");
    }

    #[test]
    fn scm_typescript_query_compiles() {
        make_extractor("typescript");
    }

    #[test]
    fn scm_tsx_query_compiles() {
        make_extractor("tsx");
    }

    #[test]
    fn scm_json_query_compiles() {
        make_extractor("json");
    }

    #[test]
    fn scm_css_query_compiles() {
        make_extractor("css");
    }

    #[test]
    fn scm_ruby_query_compiles() {
        make_extractor("ruby");
    }

    #[test]
    fn scm_lua_query_compiles() {
        make_extractor("lua");
    }

    #[test]
    fn scm_swift_query_compiles() {
        make_extractor("swift");
    }

    #[test]
    fn scm_perl_query_compiles() {
        make_extractor("perl");
    }

    #[test]
    fn scm_elixir_query_compiles() {
        make_extractor("elixir");
    }

    #[test]
    fn scm_erlang_query_compiles() {
        make_extractor("erlang");
    }

    #[test]
    fn scm_kotlin_query_compiles() {
        make_extractor("kotlin");
    }

    #[test]
    fn scm_dart_query_compiles() {
        make_extractor("dart");
    }

    #[test]
    fn scm_xml_query_compiles() {
        make_extractor("xml");
    }

    #[test]
    fn scm_nix_query_compiles() {
        make_extractor("nix");
    }

    #[test]
    fn scm_fortran_query_compiles() {
        make_extractor("fortran");
    }

    #[test]
    fn scm_gleam_query_compiles() {
        make_extractor("gleam");
    }

    /// Verify that ALL .scm query files compile against their grammars.
    #[test]
    fn scm_all_queries_compile() {
        let extractors = build_scm_extractors().expect("build_scm_extractors failed");
        assert!(!extractors.is_empty(), "no extractors built");
    }

    // ===================================================================
    // ===================================================================
    // Full coverage tests per language
    // ===================================================================

    #[test]
    fn scm_bash_full_coverage() {
        let src = "#!/bin/bash\nMAX_RETRIES=3\ngreet() { echo hi; }\nfunction deploy { echo go; }";
        let scm = make_extractor("bash").extract("t.sh", src).unwrap();
        let names: Vec<(&str, &str)> = scm
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();
        assert!(
            names.contains(&("greet", "function")),
            "missing greet: {names:?}"
        );
        assert!(
            names.contains(&("deploy", "function")),
            "missing deploy: {names:?}"
        );
        assert!(
            names.contains(&("MAX_RETRIES", "constant")),
            "missing MAX_RETRIES: {names:?}"
        );
    }

    #[test]
    fn scm_java_full_coverage() {
        let src = r#"
public class Svc {
    private static final int MAX = 3;
    public void run() {}
    public Svc() {}
}
public interface Repo { void save(); }
public enum Color { RED; }
"#;
        let scm = make_extractor("java").extract("T.java", src).unwrap();
        let names: Vec<(&str, &str)> = scm
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();
        assert!(names.contains(&("Svc", "class")), "missing Svc: {names:?}");
        assert!(names.contains(&("run", "method")), "missing run: {names:?}");
        assert!(names.contains(&("Repo", "type")), "missing Repo: {names:?}");
        assert!(
            names.contains(&("Color", "type")),
            "missing Color: {names:?}"
        );
    }

    #[test]
    fn scm_python_full_coverage() {
        let src = r#"
MAX = 3
class Svc:
    """Doc."""
    @deco
    def method(self):
        """Method doc."""
        pass
def top(): pass
"#;
        let scm = make_extractor("python").extract("s.py", src).unwrap();
        let names: Vec<(&str, &str)> = scm
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();
        assert!(
            names.contains(&("MAX", "constant")),
            "missing MAX: {names:?}"
        );
        assert!(names.contains(&("Svc", "class")), "missing Svc: {names:?}");
        assert!(
            names.contains(&("method", "method")),
            "missing method: {names:?}"
        );
        assert!(
            names.contains(&("top", "function")),
            "missing top: {names:?}"
        );
    }

    #[test]
    fn scm_go_full_coverage() {
        let src = r#"
package p
const V = "1"
type Server struct{}
func (s *Server) Addr() string { return "" }
func main() {}
"#;
        let scm = make_extractor("go").extract("m.go", src).unwrap();
        let names: Vec<(&str, &str)> = scm
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();
        assert!(names.contains(&("V", "constant")), "missing V: {names:?}");
        assert!(
            names.contains(&("Server", "class")),
            "missing Server: {names:?}"
        );
        assert!(
            names.contains(&("Addr", "method")),
            "missing Addr: {names:?}"
        );
        assert!(
            names.contains(&("main", "function")),
            "missing main: {names:?}"
        );
    }

    #[test]
    fn scm_php_full_coverage() {
        let src = r#"<?php
namespace App;
class Ctrl { const M = 1; public function idx() {} }
function helper() {}
"#;
        let scm = make_extractor("php").extract("c.php", src).unwrap();
        let names: Vec<(&str, &str)> = scm
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();
        assert!(
            names.contains(&("Ctrl", "class")),
            "missing Ctrl: {names:?}"
        );
        assert!(names.contains(&("M", "constant")), "missing M: {names:?}");
        assert!(
            names.contains(&("helper", "function")),
            "missing helper: {names:?}"
        );
    }

    #[test]
    fn scm_javascript_full_coverage() {
        let src = r#"
class Svc { method() {} }
function top() {}
const handler = () => {};
const MAX = 42;
"#;
        let scm = make_extractor("javascript").extract("a.js", src).unwrap();
        let names: Vec<(&str, &str)> = scm
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();
        assert!(names.contains(&("Svc", "class")), "missing Svc: {names:?}");
        assert!(
            names.contains(&("method", "method")),
            "missing method: {names:?}"
        );
        assert!(
            names.contains(&("top", "function")),
            "missing top: {names:?}"
        );
        assert!(
            names.contains(&("handler", "function")),
            "missing handler: {names:?}"
        );
        assert!(
            names.contains(&("MAX", "constant")),
            "missing MAX: {names:?}"
        );
    }

    #[test]
    fn scm_rust_full_coverage() {
        let src = r#"
mod inner {}
struct Foo;
enum Bar { A }
trait Baz { fn required(&self); }
impl Foo { fn new() -> Self { Foo } }
fn free() {}
const C: i32 = 1;
"#;
        let scm = make_extractor("rust_lang").extract("l.rs", src).unwrap();
        let names: Vec<(&str, &str)> = scm
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();
        assert!(names.contains(&("Foo", "class")), "missing Foo: {names:?}");
        assert!(names.contains(&("Bar", "type")), "missing Bar: {names:?}");
        assert!(names.contains(&("Baz", "type")), "missing Baz: {names:?}");
        assert!(
            names.contains(&("new", "function")),
            "missing new: {names:?}"
        );
        assert!(
            names.contains(&("free", "function")),
            "missing free: {names:?}"
        );
        assert!(names.contains(&("C", "constant")), "missing C: {names:?}");
    }

    #[test]
    fn scm_cpp_full_coverage() {
        let src = r#"
namespace ns { void helper() {} }
class Widget { void method(); };
void Widget::method() {}
"#;
        let scm = make_extractor("cpp").extract("w.cpp", src).unwrap();
        let names: Vec<(&str, &str)> = scm
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();
        assert!(
            names.contains(&("helper", "function")),
            "missing helper: {names:?}"
        );
        assert!(
            names.contains(&("Widget", "class")),
            "missing Widget: {names:?}"
        );
        assert!(
            names.contains(&("Widget::method", "function")),
            "missing Widget::method: {names:?}"
        );
    }

    #[test]
    fn scm_ruby_full_coverage() {
        let src = r#"
class Dog
  def bark; end
end
def top; end
"#;
        let scm = make_extractor("ruby").extract("d.rb", src).unwrap();
        let names: Vec<(&str, &str)> = scm
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();
        assert!(names.contains(&("Dog", "class")), "missing Dog: {names:?}");
        assert!(
            names.contains(&("bark", "method")),
            "missing bark: {names:?}"
        );
    }

    #[test]
    fn scm_perl_full_coverage() {
        let src = r#"
package Animal;
use strict;
use constant MAX_LEGS => 4;
use constant KINGDOM => 'Animalia';

sub new {
    my ($class, %args) = @_;
    return bless \%args, $class;
}

sub describe {
    my ($self) = @_;
    return "An animal";
}
1;
"#;
        let scm = make_extractor("perl").extract("Animal.pm", src).unwrap();
        let names: Vec<(&str, &str)> = scm
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();
        assert!(
            names.contains(&("Animal", "class")),
            "missing Animal: {names:?}"
        );
        assert!(
            names.contains(&("new", "function")),
            "missing new: {names:?}"
        );
        assert!(
            names.contains(&("describe", "function")),
            "missing describe: {names:?}"
        );
        assert!(
            names.contains(&("MAX_LEGS", "constant")),
            "missing MAX_LEGS: {names:?}"
        );
        assert!(
            names.contains(&("KINGDOM", "constant")),
            "missing KINGDOM: {names:?}"
        );
    }

    #[test]
    fn scm_elixir_full_coverage() {
        let src = r#"defmodule MyApp.Calculator do
  @moduledoc """
  A simple calculator module.
  """

  @type result :: {:ok, number()} | {:error, String.t()}

  @doc """
  Adds two numbers together.
  """
  def add(a, b) do
    a + b
  end

  @doc false
  defp validate(x) when is_number(x) do
    {:ok, x}
  end

  defmacro debug(expr) do
    quote do: IO.inspect(unquote(expr))
  end
end

defmodule MyApp.Types do
  @type name :: String.t()
  defguard is_positive(x) when is_number(x) and x > 0
end

defprotocol MyApp.Printable do
  @callback render(term()) :: String.t()
  def to_string(value)
end

defimpl MyApp.Printable, for: Integer do
  def to_string(value), do: Integer.to_string(value)
end
"#;
        let scm = make_extractor("elixir").extract("sample.ex", src).unwrap();

        // Module (defmodule -> class)
        let calc = scm
            .iter()
            .find(|s| s.name == "MyApp.Calculator")
            .expect("missing MyApp.Calculator");
        assert_eq!(calc.kind.as_str(), "class", "Calculator should be class");
        assert!(
            calc.summary.to_lowercase().contains("simple calculator"),
            "Calculator summary should mention 'simple calculator', got: {:?}",
            calc.summary
        );

        // Method inside module (def -> method)
        let add = scm.iter().find(|s| s.name == "add").expect("missing add");
        assert_eq!(add.kind.as_str(), "method", "add should be method");
        assert_eq!(add.qualified_name, "MyApp.Calculator.add");
        assert!(
            add.summary.contains("Adds two numbers"),
            "add summary should contain 'Adds two numbers', got: {:?}",
            add.summary
        );

        // Private method (defp -> method)
        let validate = scm
            .iter()
            .find(|s| s.name == "validate")
            .expect("missing validate");
        assert_eq!(
            validate.kind.as_str(),
            "method",
            "validate should be method"
        );
        assert_eq!(validate.qualified_name, "MyApp.Calculator.validate");

        // Macro (defmacro -> method)
        let debug = scm
            .iter()
            .find(|s| s.name == "debug")
            .expect("missing debug");
        assert_eq!(
            debug.kind.as_str(),
            "method",
            "debug macro should be method"
        );

        // Type alias (@type -> type)
        let result_type = scm
            .iter()
            .find(|s| s.name == "result")
            .expect("missing result type");
        assert_eq!(result_type.kind.as_str(), "type", "result should be type");
        assert_eq!(result_type.qualified_name, "MyApp.Calculator.result");

        // Guard (defguard in separate module)
        let guard = scm
            .iter()
            .find(|s| s.name == "is_positive")
            .expect("missing is_positive");
        assert_eq!(guard.kind.as_str(), "method", "guard should be method");
        assert_eq!(guard.qualified_name, "MyApp.Types.is_positive");

        // @type in Types module
        let name_type = scm
            .iter()
            .find(|s| s.name == "name")
            .expect("missing name type");
        assert_eq!(name_type.kind.as_str(), "type", "name should be type");

        // Protocol (defprotocol -> type)
        let protocol = scm
            .iter()
            .find(|s| s.name == "MyApp.Printable")
            .expect("missing MyApp.Printable");
        assert_eq!(protocol.kind.as_str(), "type", "protocol should be type");

        // @callback inside protocol -> type
        let callback = scm
            .iter()
            .find(|s| s.name == "render")
            .expect("missing render callback");
        assert_eq!(callback.kind.as_str(), "type", "callback should be type");

        // Protocol implementation (defimpl -> class)
        let _impl_sym = scm
            .iter()
            .find(|s| s.qualified_name.contains("Printable") && s.kind.as_str() == "class")
            .expect("missing Printable impl class");

        // Function inside impl (method)
        let to_str = scm
            .iter()
            .find(|s| s.name == "to_string")
            .expect("missing to_string");
        assert_eq!(to_str.kind.as_str(), "method", "to_string should be method");
    }

    #[test]
    fn scm_erlang_full_coverage() {
        let src = r#"-module(mymod).
-define(MAX_ITER, 100).
-record(point, {x = 0, y = 0}).
-type number_pair() :: {number(), number()}.

add(X, Y) ->
    X + Y.

factorial(0) ->
    1;
factorial(N) when N > 0 ->
    N * factorial(N - 1).
"#;
        let scm = make_extractor("erlang").extract("mymod.erl", src).unwrap();
        let names: Vec<(&str, &str)> = scm
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();
        assert!(
            names.contains(&("add", "function")),
            "missing add: {names:?}"
        );
        assert!(
            names.contains(&("factorial", "function")),
            "missing factorial: {names:?}"
        );
        assert!(
            names.contains(&("number_pair", "type")),
            "missing number_pair: {names:?}"
        );
        assert!(
            names.contains(&("point", "type")),
            "missing point: {names:?}"
        );
        assert!(
            names.contains(&("MAX_ITER", "constant")),
            "missing MAX_ITER: {names:?}"
        );
    }

    #[test]
    fn scm_swift_full_coverage() {
        let src = r#"
class Svc { func run() {} }
func helper() -> Bool { true }
protocol P { func p() }
"#;
        let scm = make_extractor("swift").extract("s.swift", src).unwrap();
        let names: Vec<(&str, &str)> = scm
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();
        assert!(names.contains(&("Svc", "class")), "missing Svc: {names:?}");
        assert!(
            names.contains(&("run", "function")),
            "missing run: {names:?}"
        );
        assert!(
            names.contains(&("helper", "function")),
            "missing helper: {names:?}"
        );
        assert!(names.contains(&("P", "type")), "missing P: {names:?}");
    }

    #[test]
    fn scm_kotlin_full_coverage() {
        let src = r#"
fun greet(name: String): String {
    return "Hello, $name!"
}

class Calculator {
    fun add(a: Int, b: Int): Int {
        return a + b
    }
}

interface Clickable {
    fun click()
}

object AppConfig {
    val version = "1.0"
}

typealias StringList = List<String>

enum class Direction {
    NORTH, SOUTH, EAST, WEST
}

data class Point(val x: Int, val y: Int)
"#;
        let scm = make_extractor("kotlin").extract("main.kt", src).unwrap();
        let names: Vec<(&str, &str)> = scm
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();
        assert!(
            names.contains(&("greet", "function")),
            "missing greet: {names:?}"
        );
        assert!(
            names.contains(&("Calculator", "class")),
            "missing Calculator: {names:?}"
        );
        assert!(names.contains(&("add", "method")), "missing add: {names:?}");
        assert!(
            names.contains(&("Clickable", "type")),
            "missing Clickable: {names:?}"
        );
        assert!(
            names.contains(&("AppConfig", "class")),
            "missing AppConfig: {names:?}"
        );
        assert!(
            names.contains(&("StringList", "type")),
            "missing StringList: {names:?}"
        );
        assert!(
            names.contains(&("Direction", "class")),
            "missing Direction: {names:?}"
        );
        assert!(
            names.contains(&("Point", "class")),
            "missing Point: {names:?}"
        );

        // Check containment-based scoping
        let method = scm.iter().find(|s| s.name == "add").unwrap();
        assert_eq!(method.kind, SymbolKind::Method);
        assert_eq!(method.qualified_name, "Calculator.add");
    }

    #[test]
    fn scm_fortran_full_coverage() {
        let src = concat!(
            "! Computes the sum of two integers.\n",
            "function add(a, b) result(res)\n",
            "  integer, intent(in) :: a, b\n",
            "  integer :: res\n",
            "  res = a + b\n",
            "end function add\n",
            "\n",
            "! Greets the user.\n",
            "subroutine greet(name)\n",
            "  character(len=*), intent(in) :: name\n",
            "  print *, \"Hello, \" // name\n",
            "end subroutine greet\n",
            "\n",
            "module math_utils\n",
            "  implicit none\n",
            "  integer, parameter :: MAX_SIZE = 100\n",
            "  type :: Point\n",
            "    real :: x\n",
            "    real :: y\n",
            "  end type Point\n",
            "contains\n",
            "  ! Multiplies two reals.\n",
            "  function multiply(a, b) result(res)\n",
            "    real, intent(in) :: a, b\n",
            "    real :: res\n",
            "    res = a * b\n",
            "  end function multiply\n",
            "end module math_utils\n",
        );
        let scm = make_extractor("fortran").extract("math.f90", src).unwrap();
        let names: Vec<(&str, &str)> = scm
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();

        // Top-level function
        assert!(
            names.contains(&("add", "function")),
            "missing add: {names:?}"
        );
        let add = scm.iter().find(|s| s.name == "add").unwrap();
        assert!(add.signature.contains("add"), "sig: {}", add.signature);
        assert!(
            add.summary.contains("Computes the sum"),
            "doc: {}",
            add.summary
        );
        assert_eq!(add.language, "fortran");

        // Top-level subroutine (as function kind)
        assert!(
            names.contains(&("greet", "function")),
            "missing greet: {names:?}"
        );
        let greet = scm.iter().find(|s| s.name == "greet").unwrap();
        assert!(
            greet.signature.contains("subroutine"),
            "sig: {}",
            greet.signature
        );
        assert!(
            greet.summary.contains("Greets the user"),
            "doc: {}",
            greet.summary
        );

        // Module (as class kind)
        assert!(
            names.contains(&("math_utils", "class")),
            "missing math_utils: {names:?}"
        );
        let module = scm.iter().find(|s| s.name == "math_utils").unwrap();
        assert!(
            module.signature.contains("module"),
            "sig: {}",
            module.signature
        );

        // Method inside module
        assert!(
            names.contains(&("multiply", "method")),
            "missing multiply: {names:?}"
        );
        let mul = scm.iter().find(|s| s.name == "multiply").unwrap();
        assert_eq!(mul.qualified_name, "math_utils.multiply");
        assert!(
            mul.summary.contains("Multiplies two reals"),
            "doc: {}",
            mul.summary
        );

        // Derived type
        assert!(
            names.contains(&("Point", "type")),
            "missing Point: {names:?}"
        );
        let pt = scm.iter().find(|s| s.name == "Point").unwrap();
        assert!(pt.signature.contains("Point"), "sig: {}", pt.signature);

        // Parameter constant
        assert!(
            names.contains(&("MAX_SIZE", "constant")),
            "missing MAX_SIZE: {names:?}"
        );
        let cst = scm.iter().find(|s| s.name == "MAX_SIZE").unwrap();
        assert_eq!(cst.qualified_name, "math_utils.MAX_SIZE");

        // Byte offsets valid
        let source_len = src.len();
        for sym in &scm {
            assert!(
                sym.end_byte <= source_len,
                "byte offsets out of range for {}",
                sym.name
            );
        }
    }

    #[test]
    fn scm_gleam_full_coverage() {
        let src = r#"pub type Color {
  Red
  Green
  Blue
}

pub type Alias = String

pub const max_size: Int = 100

// Greet a user
pub fn greet(name: String) -> String {
  "Hello, " <> name
}

fn helper(x: Int) -> Int {
  x + 1
}
"#;
        let scm = make_extractor("gleam").extract("app.gleam", src).unwrap();
        let names: Vec<(&str, &str)> = scm
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect();

        // type definition: Color
        assert!(
            names.contains(&("Color", "type")),
            "missing Color: {names:?}"
        );
        let color = scm.iter().find(|s| s.name == "Color").unwrap();
        assert!(
            color.signature.contains("Color"),
            "Color signature missing name: {:?}",
            color.signature
        );

        // type alias: Alias
        assert!(
            names.contains(&("Alias", "type")),
            "missing Alias: {names:?}"
        );

        // constant: max_size
        assert!(
            names.contains(&("max_size", "constant")),
            "missing max_size: {names:?}"
        );

        // public function: greet
        assert!(
            names.contains(&("greet", "function")),
            "missing greet: {names:?}"
        );
        let greet = scm.iter().find(|s| s.name == "greet").unwrap();
        assert!(
            greet.signature.contains("String"),
            "greet signature missing String: {:?}",
            greet.signature
        );
        assert!(
            greet.summary.contains("Greet a user"),
            "greet summary missing 'Greet a user': {:?}",
            greet.summary,
        );

        // private function: helper
        assert!(
            names.contains(&("helper", "function")),
            "missing helper: {names:?}"
        );
    }
}
