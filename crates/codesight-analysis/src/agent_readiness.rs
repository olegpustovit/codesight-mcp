//! Agent-readiness scoring based on navigability, comprehensibility,
//! verifiability, and change safety.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::complexity::cyclomatic_complexity;
use codesight_storage::storage::AnalysisSymbol;

#[derive(Clone, Copy, Debug)]
pub struct AgentReadinessDimensions {
    pub navigability: f64,
    pub comprehensibility: f64,
    pub verifiability: f64,
    pub change_safety: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct AgentReadinessSubmetrics {
    pub symbol_uniqueness: f64,
    pub structural_localization: f64,
    pub nesting_depth_health: f64,
    pub function_length_health: f64,
    pub type_annotation_density: f64,
    pub test_oracle_quality: f64,
    pub change_locality: f64,
    pub interface_encapsulation: f64,
    pub hotspot_containment: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct AgentReadinessSignal {
    pub score: u32,
    pub dimensions: AgentReadinessDimensions,
    pub submetrics: AgentReadinessSubmetrics,
    pub confidence: &'static str,
    pub type_annotation_method: &'static str,
    pub bottleneck: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LanguageFamily {
    BracedStatic,
    ScriptedDynamic,
    FunctionalExpression,
    DeclarativeMarkup,
    Unknown,
}

#[derive(Clone, Copy, Debug)]
enum NestingMode {
    Brace,
    Indent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TypeAnnotationMethod {
    Measured,
    BaselineFallback,
}

impl TypeAnnotationMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Measured => "measured",
            Self::BaselineFallback => "baseline_fallback",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct LanguageProfile {
    language: &'static str,
    family: LanguageFamily,
    nesting_mode: NestingMode,
}

impl LanguageProfile {
    fn function_length_warn(self) -> Option<u32> {
        match self.family {
            LanguageFamily::BracedStatic => Some(75),
            LanguageFamily::ScriptedDynamic => Some(40),
            LanguageFamily::FunctionalExpression => Some(30),
            LanguageFamily::DeclarativeMarkup | LanguageFamily::Unknown => None,
        }
    }

    fn fallback_type_baseline(self) -> Option<f64> {
        match self.language {
            "python" => Some(0.4),
            "typescript" | "tsx" => Some(0.7),
            "javascript" | "jsx" | "ruby" | "perl" | "lua" => Some(0.1),
            "php" => Some(0.3),
            "haskell" => Some(0.9),
            "scala" => Some(0.7),
            "elixir" | "erlang" => Some(0.3),
            "clojure" => Some(0.1),
            _ => match self.family {
                LanguageFamily::BracedStatic => Some(1.0),
                LanguageFamily::ScriptedDynamic => Some(0.3),
                LanguageFamily::FunctionalExpression => Some(0.5),
                LanguageFamily::DeclarativeMarkup | LanguageFamily::Unknown => None,
            },
        }
    }

    fn supports_measured_types(self) -> bool {
        matches!(
            self.language,
            "python"
                | "typescript"
                | "tsx"
                | "javascript"
                | "jsx"
                | "php"
                | "rust"
                | "go"
                | "java"
                | "csharp"
                | "cpp"
                | "kotlin"
                | "swift"
                | "dart"
                | "scala"
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct FunctionFacts<'a> {
    sym: &'a AnalysisSymbol,
    profile: LanguageProfile,
    importance: f64,
    file_fan_in: usize,
    cyclomatic: usize,
    nesting_depth: usize,
    line_span: u32,
    type_annotation_score: f64,
    type_annotation_method: TypeAnnotationMethod,
    graph_backed: bool,
    is_test: bool,
}

pub fn is_fn_method(kind: &str) -> bool {
    kind.eq_ignore_ascii_case("function") || kind.eq_ignore_ascii_case("method")
}

pub fn is_test_file(path: &str) -> bool {
    const DIR_MARKERS: &[&str] = &["/tests/", "/__tests__/", "/spec/", "/test/"];
    const SUFFIXES: &[&str] = &[
        "_test.rs",
        "_test.py",
        "_test.go",
        "_test.ts",
        "_test.js",
        ".test.ts",
        ".test.js",
        ".test.tsx",
        ".test.jsx",
        "_spec.rb",
        "_spec.ts",
        "_spec.js",
    ];
    let lower = path.to_ascii_lowercase();
    DIR_MARKERS.iter().any(|m| lower.contains(m))
        || SUFFIXES.iter().any(|s| lower.ends_with(s))
        || lower.starts_with("tests/")
        || lower.starts_with("test/")
}

pub fn tested_file_set(
    all_symbols: &[AnalysisSymbol],
    adj: &HashMap<String, Vec<String>>,
) -> HashSet<String> {
    let mut tested: HashSet<String> = HashSet::new();

    for (src, targets) in adj {
        if is_test_file(src) {
            for tgt in targets {
                tested.insert(tgt.clone());
            }
        }
    }

    for sym in all_symbols {
        if sym.id.contains("::tests.") {
            tested.insert(sym.file_path.clone());
        }
    }

    tested
}

pub fn compute_fan_in(adj: &HashMap<String, Vec<String>>) -> HashMap<String, usize> {
    let mut fan_in: HashMap<String, usize> = HashMap::new();
    for src in adj.keys() {
        fan_in.entry(src.clone()).or_insert(0);
    }
    for targets in adj.values() {
        for tgt in targets {
            *fan_in.entry(tgt.clone()).or_insert(0) += 1;
        }
    }
    fan_in
}

pub fn compute_file_churn(root: &Path) -> HashMap<String, u64> {
    let mut churn = HashMap::<String, u64>::new();
    if !root.join(".git").exists() {
        return churn;
    }
    let Ok(output) = std::process::Command::new("git")
        .args(["log", "--format=", "--name-only"])
        .current_dir(root)
        .output()
    else {
        return churn;
    };
    if !output.status.success() {
        return churn;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            *churn.entry(trimmed.to_string()).or_default() += 1;
        }
    }
    churn
}

pub fn compute_agent_readiness(
    all_symbols: &[AnalysisSymbol],
    adj: &HashMap<String, Vec<String>>,
    root: Option<&Path>,
) -> AgentReadinessSignal {
    let churn = root.map(compute_file_churn).unwrap_or_default();
    let graph_files = graph_files(adj);
    let fan_in = compute_fan_in(adj);
    let facts = build_function_facts(all_symbols, &fan_in, &graph_files, &churn);

    if facts.is_empty() {
        return AgentReadinessSignal {
            score: 0,
            dimensions: AgentReadinessDimensions {
                navigability: 0.0,
                comprehensibility: 0.0,
                verifiability: 0.0,
                change_safety: 0.0,
            },
            submetrics: AgentReadinessSubmetrics {
                symbol_uniqueness: 0.0,
                structural_localization: 0.0,
                nesting_depth_health: 0.0,
                function_length_health: 0.0,
                type_annotation_density: 0.0,
                test_oracle_quality: 0.0,
                change_locality: 0.0,
                interface_encapsulation: 0.0,
                hotspot_containment: 0.0,
            },
            confidence: "low",
            type_annotation_method: "baseline_fallback",
            bottleneck: "navigability",
        };
    }

    let submetrics = AgentReadinessSubmetrics {
        symbol_uniqueness: compute_symbol_uniqueness(all_symbols, &fan_in, &churn),
        structural_localization: compute_structural_localization(&facts),
        nesting_depth_health: weighted_health(&facts, |fact| fact.nesting_depth <= 4),
        function_length_health: weighted_health(&facts, |fact| {
            fact.profile
                .function_length_warn()
                .is_some_and(|limit| fact.line_span <= limit)
        }),
        type_annotation_density: weighted_type_density(&facts),
        test_oracle_quality: compute_test_oracle_quality(all_symbols, adj, &facts),
        change_locality: compute_change_locality(&graph_files, &fan_in, &churn),
        interface_encapsulation: compute_interface_encapsulation(all_symbols),
        hotspot_containment: compute_hotspot_containment(&facts),
    };

    let dimensions = AgentReadinessDimensions {
        navigability: geometric_mean(&[
            submetrics.symbol_uniqueness,
            submetrics.structural_localization,
        ]),
        comprehensibility: geometric_mean(&[
            submetrics.nesting_depth_health,
            submetrics.function_length_health,
        ]),
        // Substitute model: types are the primary verification channel.
        // Tests fill the gap where types are missing. Research shows they
        // are partial substitutes (Microsoft/DeepMind, Reflexion), not
        // independent complements — the 50/50 arithmetic mean overpunished
        // strongly-typed codebases.
        verifiability: submetrics.type_annotation_density
            + (1.0 - submetrics.type_annotation_density) * submetrics.test_oracle_quality,
        change_safety: geometric_mean(&[
            submetrics.change_locality,
            submetrics.interface_encapsulation,
            submetrics.hotspot_containment,
        ]),
    };

    let score = (geometric_mean(&[
        dimensions.navigability,
        dimensions.comprehensibility,
        dimensions.verifiability,
        dimensions.change_safety,
    ]) * 10000.0)
        .round() as u32;

    let fallback_type_count = facts
        .iter()
        .filter(|fact| fact.type_annotation_method == TypeAnnotationMethod::BaselineFallback)
        .count();
    let measured_type_count = facts.len() - fallback_type_count;
    let graph_backed_function_files = facts
        .iter()
        .filter(|fact| fact.graph_backed)
        .map(|fact| fact.sym.file_path.as_str())
        .collect::<HashSet<_>>()
        .len();
    let total_function_files = facts
        .iter()
        .map(|fact| fact.sym.file_path.as_str())
        .collect::<HashSet<_>>()
        .len()
        .max(1);
    let graph_coverage_ratio = graph_backed_function_files as f64 / total_function_files as f64;
    let family_known_ratio = facts
        .iter()
        .filter(|fact| fact.profile.family != LanguageFamily::Unknown)
        .count() as f64
        / facts.len() as f64;
    let measured_type_ratio = measured_type_count as f64 / facts.len() as f64;
    let confidence = if !churn.is_empty()
        && graph_coverage_ratio >= 0.9
        && family_known_ratio >= 0.95
        && measured_type_ratio >= 0.95
    {
        "high"
    } else if graph_coverage_ratio > 0.0 || !churn.is_empty() || measured_type_count > 0 {
        "medium"
    } else {
        "low"
    };

    let type_annotation_method = if fallback_type_count == 0 {
        TypeAnnotationMethod::Measured.as_str()
    } else {
        TypeAnnotationMethod::BaselineFallback.as_str()
    };

    let bottleneck = [
        (dimensions.navigability, "navigability"),
        (dimensions.comprehensibility, "comprehensibility"),
        (dimensions.verifiability, "verifiability"),
        (dimensions.change_safety, "change_safety"),
    ]
    .into_iter()
    .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
    .map(|(_, name)| name)
    .unwrap_or("navigability");

    AgentReadinessSignal {
        score,
        dimensions,
        submetrics,
        confidence,
        type_annotation_method,
        bottleneck,
    }
}

fn build_function_facts<'a>(
    all_symbols: &'a [AnalysisSymbol],
    fan_in: &HashMap<String, usize>,
    graph_files: &HashSet<String>,
    churn: &HashMap<String, u64>,
) -> Vec<FunctionFacts<'a>> {
    all_symbols
        .iter()
        .filter(|sym| is_fn_method(&sym.kind))
        .filter_map(|sym| {
            let profile = language_profile(&sym.file_path);
            if matches!(
                profile.family,
                LanguageFamily::DeclarativeMarkup | LanguageFamily::Unknown
            ) {
                return None;
            }
            let file_fan_in = *fan_in.get(&sym.file_path).unwrap_or(&0);
            let file_churn = *churn.get(&sym.file_path).unwrap_or(&0);
            let importance = importance_weight(file_fan_in, file_churn);
            let cyclomatic = cyclomatic_complexity(&sym.source_text);
            let nesting_depth = max_nesting_depth(&sym.source_text, profile);
            let line_span = sym.end_line.saturating_sub(sym.start_line) + 1;
            let (type_annotation_score, type_annotation_method) =
                measure_type_annotation_density(sym, profile);

            Some(FunctionFacts {
                sym,
                profile,
                importance,
                file_fan_in,
                cyclomatic,
                nesting_depth,
                line_span,
                type_annotation_score,
                type_annotation_method,
                graph_backed: graph_files.contains(&sym.file_path),
                is_test: is_test_symbol(sym),
            })
        })
        .collect()
}

fn compute_symbol_uniqueness(
    all_symbols: &[AnalysisSymbol],
    fan_in: &HashMap<String, usize>,
    churn: &HashMap<String, u64>,
) -> f64 {
    let important: Vec<&AnalysisSymbol> = all_symbols
        .iter()
        .filter(|sym| is_navigability_symbol(&sym.kind))
        .filter(|sym| !is_test_symbol(sym))
        .collect();
    if important.is_empty() {
        return 1.0;
    }

    let mut name_files: HashMap<&str, HashSet<&str>> = HashMap::new();
    for sym in &important {
        name_files
            .entry(sym.name.as_str())
            .or_default()
            .insert(sym.file_path.as_str());
    }

    let mut weighted_ambiguity_sum = 0.0;
    let mut weight_sum = 0.0;
    for sym in important {
        let importance = importance_weight(
            *fan_in.get(&sym.file_path).unwrap_or(&0),
            *churn.get(&sym.file_path).unwrap_or(&0),
        );
        let variants = name_files
            .get(sym.name.as_str())
            .map(|files| files.len())
            .unwrap_or(1);
        let ambiguity = variants.saturating_sub(1) as f64;
        weighted_ambiguity_sum += ambiguity * importance;
        weight_sum += importance;
    }

    if weight_sum == 0.0 {
        return 1.0;
    }
    let weighted_mean = weighted_ambiguity_sum / weight_sum;
    (1.0 / (1.0 + weighted_mean)).clamp(0.0, 1.0)
}

fn compute_structural_localization(facts: &[FunctionFacts<'_>]) -> f64 {
    let mut importer_counts: Vec<usize> = facts
        .iter()
        .filter(|fact| !fact.is_test && fact.graph_backed && fact.file_fan_in > 0)
        .map(|fact| fact.file_fan_in)
        .collect();
    if importer_counts.is_empty() {
        return 1.0;
    }
    importer_counts.sort_unstable();
    let mid = importer_counts.len() / 2;
    let median = if importer_counts.len().is_multiple_of(2) {
        (importer_counts[mid - 1] + importer_counts[mid]) as f64 / 2.0
    } else {
        importer_counts[mid] as f64
    };
    (1.0 / (1.0 + median / 3.0)).clamp(0.0, 1.0)
}

fn weighted_health<F>(facts: &[FunctionFacts<'_>], predicate: F) -> f64
where
    F: Fn(&FunctionFacts<'_>) -> bool,
{
    let mut weight_sum = 0.0;
    let mut healthy_weight = 0.0;
    for fact in facts {
        if fact.is_test {
            continue;
        }
        weight_sum += fact.importance;
        if predicate(fact) {
            healthy_weight += fact.importance;
        }
    }
    if weight_sum == 0.0 {
        return 1.0;
    }
    (healthy_weight / weight_sum).clamp(0.0, 1.0)
}

fn weighted_type_density(facts: &[FunctionFacts<'_>]) -> f64 {
    let mut weighted_sum = 0.0;
    let mut weight_sum = 0.0;
    for fact in facts {
        if fact.is_test {
            continue;
        }
        weighted_sum += fact.type_annotation_score * fact.importance;
        weight_sum += fact.importance;
    }
    if weight_sum == 0.0 {
        return 0.0;
    }
    (weighted_sum / weight_sum).clamp(0.0, 1.0)
}

fn compute_test_oracle_quality(
    all_symbols: &[AnalysisSymbol],
    adj: &HashMap<String, Vec<String>>,
    facts: &[FunctionFacts<'_>],
) -> f64 {
    let mut source_function_counts: HashMap<&str, usize> = HashMap::new();
    let mut source_weights: HashMap<&str, f64> = HashMap::new();
    let mut inline_test_counts: HashMap<&str, usize> = HashMap::new();
    let mut external_test_counts: HashMap<String, f64> = HashMap::new();
    let mut test_file_function_counts: HashMap<&str, usize> = HashMap::new();

    for fact in facts {
        if fact.is_test {
            continue;
        }
        *source_function_counts
            .entry(fact.sym.file_path.as_str())
            .or_default() += 1;
        *source_weights
            .entry(fact.sym.file_path.as_str())
            .or_default() += fact.importance;
    }
    if source_function_counts.is_empty() {
        return 0.0;
    }

    for sym in all_symbols {
        if !is_fn_method(&sym.kind) {
            continue;
        }
        if is_test_file(&sym.file_path) {
            *test_file_function_counts
                .entry(sym.file_path.as_str())
                .or_default() += 1;
        } else if sym.id.contains("::tests.") {
            *inline_test_counts
                .entry(sym.file_path.as_str())
                .or_default() += 1;
        }
    }

    for (test_file, targets) in adj {
        if !is_test_file(test_file) {
            continue;
        }
        let test_fn_count = *test_file_function_counts
            .get(test_file.as_str())
            .unwrap_or(&0);
        if test_fn_count == 0 {
            continue;
        }
        let covered: Vec<&str> = targets
            .iter()
            .map(String::as_str)
            .filter(|target| source_function_counts.contains_key(target))
            .collect();
        if covered.is_empty() {
            continue;
        }
        let share = test_fn_count as f64 / covered.len() as f64;
        for file in covered {
            *external_test_counts.entry(file.to_string()).or_default() += share;
        }
    }

    let mut weighted_sum = 0.0;
    let mut weight_sum = 0.0;
    for (file, fn_count) in source_function_counts {
        let inline = *inline_test_counts.get(file).unwrap_or(&0) as f64;
        let external = *external_test_counts.get(file).unwrap_or(&0.0);
        let ratio = ((inline + external) / fn_count as f64).min(1.0);
        let weight = *source_weights.get(file).unwrap_or(&1.0);
        weighted_sum += ratio * weight;
        weight_sum += weight;
    }
    if weight_sum == 0.0 {
        return 0.0;
    }
    (weighted_sum / weight_sum).clamp(0.0, 1.0)
}

fn compute_change_locality(
    graph_files: &HashSet<String>,
    fan_in: &HashMap<String, usize>,
    churn: &HashMap<String, u64>,
) -> f64 {
    if graph_files.is_empty() {
        return 1.0;
    }

    let mut weighted_surface = 0.0;
    let mut file_count = 0usize;

    for file in graph_files {
        let importers = *fan_in.get(file).unwrap_or(&0) as f64;
        let file_churn = *churn.get(file).unwrap_or(&0) as f64;
        if importers < 2.0 && file_churn < 5.0 {
            continue;
        }
        let surface = (1.0 + importers) * (1.0 + (1.0 + file_churn).ln());
        weighted_surface += surface;
        file_count += 1;
    }

    if file_count == 0 {
        return 1.0;
    }

    let avg_surface = weighted_surface / file_count as f64;
    (1.0 / (1.0 + avg_surface / 8.0)).clamp(0.0, 1.0)
}

fn compute_interface_encapsulation(all_symbols: &[AnalysisSymbol]) -> f64 {
    let mut total = 0usize;
    let mut restricted = 0usize;
    for sym in all_symbols {
        if is_test_symbol(sym) {
            continue;
        }
        total += 1;
        let profile = language_profile(&sym.file_path);
        if has_restricted_visibility(sym, profile.language) {
            restricted += 1;
        }
    }
    if total == 0 {
        return 1.0;
    }
    (restricted as f64 / total as f64).clamp(0.0, 1.0)
}

fn compute_hotspot_containment(facts: &[FunctionFacts<'_>]) -> f64 {
    let hotspots: Vec<&FunctionFacts<'_>> = facts
        .iter()
        .filter(|fact| !fact.is_test && fact.graph_backed && fact.cyclomatic > 15)
        .collect();
    if hotspots.is_empty() {
        return 1.0;
    }
    let contained = hotspots.iter().filter(|fact| fact.file_fan_in <= 5).count();
    (contained as f64 / hotspots.len() as f64).clamp(0.0, 1.0)
}

fn graph_files(adj: &HashMap<String, Vec<String>>) -> HashSet<String> {
    let mut files = HashSet::new();
    for (src, targets) in adj {
        files.insert(src.clone());
        for tgt in targets {
            files.insert(tgt.clone());
        }
    }
    files
}

fn importance_weight(fan_in: usize, churn: u64) -> f64 {
    (1.0 + fan_in as f64) * (1.0 + (1.0 + churn as f64).ln())
}

fn geometric_mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let product: f64 = values.iter().product();
    product.powf(1.0 / values.len() as f64)
}

fn is_navigability_symbol(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "function" | "method" | "class" | "struct" | "trait" | "interface" | "enum"
    )
}

fn is_test_symbol(sym: &AnalysisSymbol) -> bool {
    is_test_file(&sym.file_path) || sym.id.contains("::tests.")
}

fn language_profile(file_path: &str) -> LanguageProfile {
    let lower = file_path.to_ascii_lowercase();
    let language = if lower.ends_with(".blade.php") {
        "blade"
    } else if lower.ends_with(".cshtml") || lower.ends_with(".razor") {
        "razor"
    } else if lower.ends_with("makefile") || lower.ends_with("/makefile") {
        "makefile"
    } else {
        match lower.rsplit('.').next().unwrap_or("") {
            "rs" => "rust",
            "go" => "go",
            "java" => "java",
            "cs" => "csharp",
            "cc" | "cp" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => "cpp",
            "swift" => "swift",
            "kt" | "kts" => "kotlin",
            "dart" => "dart",
            "py" | "pyi" => "python",
            "rb" => "ruby",
            "js" | "mjs" | "cjs" => "javascript",
            "ts" | "mts" | "cts" => "typescript",
            "tsx" => "tsx",
            "jsx" => "jsx",
            "php" => "php",
            "pl" | "pm" => "perl",
            "lua" => "lua",
            "ex" | "exs" => "elixir",
            "erl" | "hrl" => "erlang",
            "hs" => "haskell",
            "ml" | "mli" => "ocaml",
            "fs" | "fsi" | "fsx" => "fsharp",
            "scala" | "sc" => "scala",
            "clj" | "cljs" | "cljc" => "clojure",
            "json" => "json",
            "xml" => "xml",
            "html" | "htm" => "html",
            "css" | "scss" | "less" => "css",
            "yaml" | "yml" => "yaml",
            "toml" => "toml",
            "sql" => "sql",
            _ => "unknown",
        }
    };

    match language {
        "rust" | "go" | "java" | "csharp" | "cpp" | "swift" | "kotlin" | "dart" => {
            LanguageProfile {
                language,
                family: LanguageFamily::BracedStatic,
                nesting_mode: NestingMode::Brace,
            }
        }
        "python" | "ruby" | "perl" | "lua" => LanguageProfile {
            language,
            family: LanguageFamily::ScriptedDynamic,
            nesting_mode: NestingMode::Indent,
        },
        "javascript" | "typescript" | "tsx" | "jsx" | "php" => LanguageProfile {
            language,
            family: LanguageFamily::ScriptedDynamic,
            nesting_mode: NestingMode::Brace,
        },
        "scala" => LanguageProfile {
            language,
            family: LanguageFamily::FunctionalExpression,
            nesting_mode: NestingMode::Brace,
        },
        "elixir" | "erlang" | "haskell" | "ocaml" | "fsharp" | "clojure" => LanguageProfile {
            language,
            family: LanguageFamily::FunctionalExpression,
            nesting_mode: NestingMode::Indent,
        },
        "json" | "xml" | "html" | "css" | "yaml" | "toml" | "sql" | "makefile" | "blade"
        | "razor" => LanguageProfile {
            language,
            family: LanguageFamily::DeclarativeMarkup,
            nesting_mode: NestingMode::Indent,
        },
        _ => LanguageProfile {
            language,
            family: LanguageFamily::Unknown,
            nesting_mode: NestingMode::Indent,
        },
    }
}

/// Compute nesting depth for a source body given its file path.
/// Exported for use in the per-symbol complexity payload.
pub fn max_nesting_depth_for_file(source: &str, file_path: &str) -> usize {
    max_nesting_depth(source, language_profile(file_path))
}

fn max_nesting_depth(source: &str, profile: LanguageProfile) -> usize {
    match profile.nesting_mode {
        NestingMode::Brace => brace_nesting_depth(source),
        NestingMode::Indent => indent_nesting_depth(source),
    }
}

fn brace_nesting_depth(source: &str) -> usize {
    let mut depth = 0usize;
    let mut max_depth = 0usize;
    let mut chars = source.chars().peekable();
    let mut in_string = None;
    let mut escape = false;
    let mut in_block_comment = false;
    let mut in_line_comment = false;

    while let Some(ch) = chars.next() {
        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
            }
            continue;
        }
        if in_block_comment {
            if ch == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block_comment = false;
            }
            continue;
        }
        if let Some(quote) = in_string {
            if escape {
                escape = false;
                continue;
            }
            if ch == '\\' {
                escape = true;
                continue;
            }
            if ch == quote {
                in_string = None;
            }
            continue;
        }

        if ch == '"' || ch == '`' {
            in_string = Some(ch);
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            chars.next();
            in_line_comment = true;
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            in_block_comment = true;
            continue;
        }
        if ch == '{' {
            depth += 1;
            max_depth = max_depth.max(depth);
        } else if ch == '}' {
            depth = depth.saturating_sub(1);
        }
    }

    max_depth
}

fn indent_nesting_depth(source: &str) -> usize {
    let mut stack = vec![0usize];
    let mut max_depth = 0usize;

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with('#')
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*')
        {
            continue;
        }

        let indent = line
            .chars()
            .take_while(|ch| *ch == ' ' || *ch == '\t')
            .fold(0usize, |acc, ch| acc + if ch == '\t' { 4 } else { 1 });

        while stack.last().copied().unwrap_or(0) > indent {
            stack.pop();
        }
        if indent > stack.last().copied().unwrap_or(0) {
            stack.push(indent);
        }
        max_depth = max_depth.max(stack.len().saturating_sub(1));
    }

    max_depth
}

fn measure_type_annotation_density(
    sym: &AnalysisSymbol,
    profile: LanguageProfile,
) -> (f64, TypeAnnotationMethod) {
    if profile.supports_measured_types()
        && let Some(score) = measured_type_density(&sym.source_text, profile.language)
    {
        return (score, TypeAnnotationMethod::Measured);
    }

    (
        profile.fallback_type_baseline().unwrap_or(0.0),
        TypeAnnotationMethod::BaselineFallback,
    )
}

fn measured_type_density(source: &str, language: &str) -> Option<f64> {
    match language {
        "rust" => measure_colon_after_name(source, language, true, is_rust_receiver),
        "python" => measure_colon_after_name(source, language, false, is_python_receiver),
        "typescript" | "tsx" | "kotlin" | "swift" | "scala" => {
            measure_colon_after_name(source, language, false, |_| false)
        }
        "javascript" | "jsx" => measure_colon_after_name(source, language, false, |_| false),
        "php" => measure_php_signature(source),
        "go" => measure_go_signature(source),
        "java" | "csharp" | "cpp" | "dart" => measure_type_before_name(source, language),
        _ => None,
    }
}

fn measure_colon_after_name<F>(
    source: &str,
    language: &str,
    implicit_unit_return: bool,
    is_receiver: F,
) -> Option<f64>
where
    F: Fn(&str) -> bool,
{
    let open = source.find('(')?;
    let close = find_matching_delimiter(source, open, '(', ')')?;
    let params = &source[open + 1..close];
    let suffix = &source[close + 1..];

    let mut total_slots = 0usize;
    let mut score_sum = 0.0;
    for raw_param in split_top_level(params, ',') {
        let param = raw_param.trim();
        if param.is_empty() || param == "/" || param == "*" || is_receiver(param) {
            continue;
        }
        total_slots += 1;
        if let Some(ty) = extract_colon_param_type(param) {
            score_sum += type_token_score(ty, language);
        }
    }

    total_slots += 1;
    if let Some(return_ty) = extract_return_type_after_params(suffix, language) {
        score_sum += type_token_score(return_ty, language);
    } else if implicit_unit_return {
        score_sum += 1.0;
    }

    Some(score_sum / total_slots as f64)
}

fn measure_php_signature(source: &str) -> Option<f64> {
    let open = source.find('(')?;
    let close = find_matching_delimiter(source, open, '(', ')')?;
    let params = &source[open + 1..close];
    let suffix = &source[close + 1..];

    let mut total_slots = 0usize;
    let mut score_sum = 0.0;
    for raw_param in split_top_level(params, ',') {
        let param = raw_param.trim();
        if param.is_empty() {
            continue;
        }
        total_slots += 1;
        let tokens: Vec<&str> = param.split_whitespace().collect();
        if let Some(var_pos) = tokens.iter().position(|token| token.starts_with('$'))
            && var_pos > 0
        {
            score_sum += type_token_score(&tokens[..var_pos].join(" "), "php");
        }
    }

    total_slots += 1;
    if let Some(return_ty) = extract_return_type_after_params(suffix, "php") {
        score_sum += type_token_score(return_ty, "php");
    }

    Some(score_sum / total_slots as f64)
}

fn measure_type_before_name(source: &str, language: &str) -> Option<f64> {
    let open = source.find('(')?;
    let close = find_matching_delimiter(source, open, '(', ')')?;
    let before_open = source[..open].trim();
    let params = &source[open + 1..close];

    let return_type = infer_prefixed_return_type(before_open)?;
    let mut total_slots = 1usize;
    let mut score_sum = type_token_score(&return_type, language);

    for raw_param in split_top_level(params, ',') {
        let param = raw_param.trim();
        if param.is_empty() {
            continue;
        }
        total_slots += 1;
        if let Some(ty) = extract_prefixed_param_type(param) {
            score_sum += type_token_score(&ty, language);
        }
    }

    Some(score_sum / total_slots as f64)
}

fn measure_go_signature(source: &str) -> Option<f64> {
    let func_idx = source.find("func")?;
    let mut scan = source[func_idx + 4..].trim_start();
    if scan.starts_with('(') {
        let open = source.len() - scan.len();
        let close = find_matching_delimiter(source, open, '(', ')')?;
        scan = source[close + 1..].trim_start();
    }

    let name_paren_offset = scan.find('(')?;
    let params_open = source.len() - scan.len() + name_paren_offset;
    let params_close = find_matching_delimiter(source, params_open, '(', ')')?;
    let params = &source[params_open + 1..params_close];
    let suffix = source[params_close + 1..].trim_start();

    let mut total_slots = 0usize;
    let mut score_sum = 0.0;
    let mut pending_names = 0usize;

    for raw_part in split_top_level(params, ',') {
        let part = raw_part.trim();
        if part.is_empty() {
            continue;
        }
        if !part.contains(' ') && !part.starts_with("...") {
            pending_names += 1;
            continue;
        }

        let Some(last_space) = part.rfind(' ') else {
            total_slots += 1 + pending_names;
            pending_names = 0;
            continue;
        };
        let names = part[..last_space]
            .split(',')
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .count()
            + pending_names;
        let ty = part[last_space + 1..].trim();
        total_slots += names.max(1);
        score_sum += type_token_score(ty, "go") * names.max(1) as f64;
        pending_names = 0;
    }
    if pending_names > 0 {
        total_slots += pending_names;
    }

    total_slots += 1;
    if suffix.starts_with('{') {
        score_sum += 1.0;
    } else {
        let return_ty = take_until_top_level_char(suffix, &['{']).trim();
        if return_ty.is_empty() {
            score_sum += 1.0;
        } else {
            score_sum += type_token_score(return_ty, "go");
        }
    }

    Some(score_sum / total_slots as f64)
}

fn extract_colon_param_type(param: &str) -> Option<&str> {
    let colon = find_top_level_colon(param)?;
    let annotation = param[colon + 1..].trim();
    let annotation = take_until_top_level_char(annotation, &['=']).trim();
    (!annotation.is_empty()).then_some(annotation)
}

fn extract_return_type_after_params<'a>(suffix: &'a str, language: &str) -> Option<&'a str> {
    let trimmed = suffix.trim_start();
    if matches!(language, "typescript" | "tsx" | "kotlin" | "scala" | "php")
        && trimmed.starts_with(':')
    {
        let ty = take_until_top_level_char(trimmed[1..].trim_start(), &['{', '=', ';'])
            .trim_end_matches(':')
            .trim();
        return (!ty.is_empty()).then_some(ty);
    }
    if let Some(idx) = find_top_level_arrow(trimmed) {
        let ty =
            take_until_top_level_char(trimmed[idx + 2..].trim_start(), &['{', ':', '=']).trim();
        return (!ty.is_empty()).then_some(ty);
    }
    None
}

fn infer_prefixed_return_type(before_open: &str) -> Option<String> {
    let trimmed = before_open.trim();
    let mut tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }
    tokens.pop()?;
    while matches!(
        tokens.first().copied(),
        Some(
            "pub"
                | "public"
                | "private"
                | "protected"
                | "internal"
                | "static"
                | "abstract"
                | "async"
                | "virtual"
                | "override"
                | "sealed"
                | "open"
                | "final"
                | "synchronized"
                | "inline"
                | "constexpr"
                | "extern"
                | "unsafe"
        )
    ) {
        tokens.remove(0);
    }
    (!tokens.is_empty()).then(|| tokens.join(" "))
}

fn extract_prefixed_param_type(param: &str) -> Option<String> {
    let trimmed = param.trim();
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    if tokens.len() == 1 {
        return None;
    }
    let mut start = 0usize;
    while matches!(
        tokens.get(start).copied(),
        Some("final" | "readonly" | "ref" | "out" | "in" | "params" | "this")
    ) {
        start += 1;
    }
    if tokens.len() <= start + 1 {
        return None;
    }
    let joined = tokens[start..tokens.len() - 1].join(" ");
    (!joined.is_empty()).then_some(joined)
}

fn type_token_score(token: &str, language: &str) -> f64 {
    let normalized = normalize_type_token(token);
    if normalized.is_empty() {
        return 0.0;
    }
    if normalized == "unknown" && matches!(language, "typescript" | "tsx") {
        return 0.3;
    }
    if normalized == "any"
        || normalized == "typing.any"
        || normalized == "object"
        || normalized == "dynamic"
        || normalized == "mixed"
        || normalized == "interface{}"
        || normalized == "void*"
        || normalized == "auto"
        || normalized == "t.untyped"
        || normalized == "term()"
        || normalized == "anyref"
        || normalized == "anyobject"
        || normalized == "dynany"
        || normalized == "box<dynany>"
        || normalized == "{*}"
        || normalized.contains("<any>")
        || normalized.contains("typing.any")
        || normalized.contains("dynany")
    {
        return 0.0;
    }
    1.0
}

fn normalize_type_token(token: &str) -> String {
    token
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '\'')
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn has_restricted_visibility(sym: &AnalysisSymbol, language: &str) -> bool {
    let trimmed = sym.source_text.trim_start();
    match language {
        "rust" => {
            if trimmed.starts_with("pub(crate)")
                || trimmed.starts_with("pub(super)")
                || trimmed.starts_with("pub(self)")
                || trimmed.starts_with("pub(in ")
            {
                true
            } else {
                !trimmed.starts_with("pub ")
            }
        }
        "go" => sym
            .name
            .chars()
            .next()
            .map(|ch| !ch.is_uppercase())
            .unwrap_or(true),
        "python" | "ruby" | "perl" | "lua" => sym.name.starts_with('_'),
        "elixir" | "erlang" => trimmed.starts_with("defp ") || trimmed.starts_with("defmacrop "),
        "javascript" | "typescript" | "tsx" | "jsx" => {
            if trimmed.starts_with("private ") || trimmed.starts_with("protected ") {
                true
            } else {
                !trimmed.starts_with("export ")
            }
        }
        "java" | "csharp" | "cpp" | "php" | "kotlin" | "swift" | "dart" | "scala" => {
            if trimmed.starts_with("private ")
                || trimmed.starts_with("protected ")
                || trimmed.starts_with("internal ")
            {
                true
            } else {
                !trimmed.starts_with("public ")
            }
        }
        _ => sym.name.starts_with('_'),
    }
}

/// Iterator that yields `(byte_offset, char)` pairs while tracking
/// string literals and `()[]{}<>` nesting depth.  The `at_top_level()`
/// predicate is `true` when all four bracket counters are zero and we
/// are not inside a string — callers use it to decide which characters
/// are structurally significant.
struct TopLevelScanner<'a> {
    inner: std::str::CharIndices<'a>,
    paren: usize,
    bracket: usize,
    brace: usize,
    angle: usize,
    in_string: Option<char>,
    escape: bool,
}

impl<'a> TopLevelScanner<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            inner: text.char_indices(),
            paren: 0,
            bracket: 0,
            brace: 0,
            angle: 0,
            in_string: None,
            escape: false,
        }
    }

    fn at_top_level(&self) -> bool {
        self.in_string.is_none()
            && self.paren == 0
            && self.bracket == 0
            && self.brace == 0
            && self.angle == 0
    }
}

impl Iterator for TopLevelScanner<'_> {
    type Item = (usize, char);

    fn next(&mut self) -> Option<(usize, char)> {
        let (idx, ch) = self.inner.next()?;

        if let Some(quote) = self.in_string {
            if self.escape {
                self.escape = false;
            } else if ch == '\\' {
                self.escape = true;
            } else if ch == quote {
                self.in_string = None;
            }
            return Some((idx, ch));
        }

        match ch {
            '"' | '`' => self.in_string = Some(ch),
            '(' => self.paren += 1,
            ')' => self.paren = self.paren.saturating_sub(1),
            '[' => self.bracket += 1,
            ']' => self.bracket = self.bracket.saturating_sub(1),
            '{' => self.brace += 1,
            '}' => self.brace = self.brace.saturating_sub(1),
            '<' => self.angle += 1,
            '>' => self.angle = self.angle.saturating_sub(1),
            _ => {}
        }
        Some((idx, ch))
    }
}

fn find_matching_delimiter(
    text: &str,
    open_index: usize,
    open: char,
    close: char,
) -> Option<usize> {
    let mut depth = 0usize;
    let mut scanner = TopLevelScanner::new(&text[open_index..]);
    while let Some((idx, ch)) = scanner.next() {
        if scanner.in_string.is_some() {
            continue;
        }
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(open_index + idx);
            }
        }
    }
    None
}

fn split_top_level(text: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut scanner = TopLevelScanner::new(text);
    while let Some((idx, ch)) = scanner.next() {
        if ch == delimiter && scanner.at_top_level() {
            parts.push(&text[start..idx]);
            start = idx + ch.len_utf8();
        }
    }
    parts.push(&text[start..]);
    parts
}

fn take_until_top_level_char<'a>(text: &'a str, stop_chars: &[char]) -> &'a str {
    let mut scanner = TopLevelScanner::new(text);
    while let Some((idx, ch)) = scanner.next() {
        if scanner.at_top_level() && stop_chars.contains(&ch) {
            return &text[..idx];
        }
    }
    text
}

fn find_top_level_colon(text: &str) -> Option<usize> {
    let mut scanner = TopLevelScanner::new(text);
    while let Some((idx, ch)) = scanner.next() {
        if ch == ':' && scanner.at_top_level() {
            return Some(idx);
        }
    }
    None
}

fn find_top_level_arrow(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut scanner = TopLevelScanner::new(text);
    while let Some((idx, ch)) = scanner.next() {
        if ch == '-' && idx + 1 < bytes.len() && bytes[idx + 1] == b'>' && scanner.at_top_level() {
            return Some(idx);
        }
    }
    None
}

fn is_python_receiver(param: &str) -> bool {
    let name = param.split(':').next().unwrap_or(param).trim();
    matches!(name, "self" | "cls" | "*self" | "*cls")
}

fn is_rust_receiver(param: &str) -> bool {
    let normalized = param.replace(' ', "");
    matches!(
        normalized.as_str(),
        "self" | "&self" | "&mutself" | "mutself" | "self:Self" | "self:Box<Self>"
    ) || normalized.starts_with("self:")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(id: &str, file: &str, source: &str) -> AnalysisSymbol {
        AnalysisSymbol {
            id: id.to_string(),
            kind: "function".to_string(),
            name: id.split("::").last().unwrap_or("f").to_string(),
            file_path: file.to_string(),
            start_line: 1,
            end_line: source.lines().count() as u32,
            source_text: source.to_string(),
        }
    }

    #[test]
    fn rust_unit_return_counts_as_typed() {
        let source = "fn run(&self, value: i32) {\n    if value > 1 {\n        println!(\"{value}\");\n    }\n}";
        let score = measured_type_density(source, "rust").unwrap();
        assert!((score - 1.0).abs() < 0.001, "unexpected score {score}");
    }

    #[test]
    fn rust_dyn_any_is_penalized() {
        let source = "fn erase(value: Box<dyn Any>) -> Box<dyn Any> { value }";
        let score = measured_type_density(source, "rust").unwrap();
        assert!(score < 0.1, "dyn Any should score near zero, got {score}");
    }

    #[test]
    fn python_any_and_missing_annotations_are_penalized() {
        let source = "def foo(x: Any, y) -> None:\n    return None\n";
        let score = measured_type_density(source, "python").unwrap();
        assert!(
            (score - (1.0 / 3.0)).abs() < 0.05,
            "unexpected score {score}"
        );
    }

    #[test]
    fn go_grouped_parameters_and_void_return_are_typed() {
        let source = "func foo(x, y int) {\n    fmt.Println(x + y)\n}";
        let score = measured_type_density(source, "go").unwrap();
        assert!((score - 1.0).abs() < 0.001, "unexpected score {score}");
    }

    #[test]
    fn javascript_without_jsdoc_scores_zero() {
        let source = "function foo(x, y) {\n  return x + y;\n}";
        let score = measured_type_density(source, "javascript").unwrap();
        assert!(score < 0.01, "unexpected score {score}");
    }

    #[test]
    fn brace_nesting_stays_flat_for_match_style_blocks() {
        let source = "fn foo(x: i32) {\n    match x {\n        1 => println!(\"one\"),\n        _ => println!(\"other\"),\n    }\n}";
        assert_eq!(brace_nesting_depth(source), 2);
    }

    #[test]
    fn indent_nesting_tracks_python_blocks() {
        let source = "def foo(x):\n    if x:\n        for item in x:\n            if item:\n                return item\n    return None\n";
        assert_eq!(indent_nesting_depth(source), 4);
    }

    #[test]
    fn tested_file_set_detects_inline_tests() {
        let syms = vec![
            sym(
                "src/foo.rs::do_work#function",
                "src/foo.rs",
                "fn do_work() {}",
            ),
            sym(
                "src/foo.rs::tests.test_do_work#function",
                "src/foo.rs",
                "fn test_do_work() {}",
            ),
            sym(
                "src/bar.rs::handle#function",
                "src/bar.rs",
                "fn handle() {}",
            ),
        ];
        let adj = HashMap::new();
        let tested = tested_file_set(&syms, &adj);
        assert!(tested.contains("src/foo.rs"));
        assert!(!tested.contains("src/bar.rs"));
    }

    #[test]
    fn tested_file_set_detects_dep_graph_edges() {
        let syms = vec![];
        let adj = HashMap::from([(
            "tests/integration.rs".to_string(),
            vec!["src/lib.rs".to_string()],
        )]);
        let tested = tested_file_set(&syms, &adj);
        assert!(tested.contains("src/lib.rs"));
    }

    #[test]
    fn verifiability_substitute_model() {
        // Types are primary; tests fill the gap where types are missing.
        // verifiability = type + (1 - type) × oracle

        // Fully typed, no tests → types cover it
        let typed_no_tests = make_verifiability(1.0, 0.0);
        assert!((typed_no_tests - 1.0).abs() < 0.001, "got {typed_no_tests}");

        // No types, good tests → tests are the only signal
        let untyped_good_tests = make_verifiability(0.0, 0.8);
        assert!(
            (untyped_good_tests - 0.8).abs() < 0.001,
            "got {untyped_good_tests}"
        );

        // Partial types, partial tests → types dominate
        let partial = make_verifiability(0.7, 0.5);
        // 0.7 + 0.3 × 0.5 = 0.85
        assert!((partial - 0.85).abs() < 0.001, "got {partial}");

        // No types, no tests → correctly terrible
        let nothing = make_verifiability(0.0, 0.0);
        assert!(nothing < 0.001, "got {nothing}");
    }

    fn make_verifiability(type_density: f64, oracle: f64) -> f64 {
        type_density + (1.0 - type_density) * oracle
    }

    #[test]
    fn interface_encapsulation_respects_rust_visibility() {
        let syms = vec![
            sym("a", "src/lib.rs", "pub fn public_api() {}"),
            sym("b", "src/lib.rs", "fn private_impl() {}"),
        ];
        let score = compute_interface_encapsulation(&syms);
        assert!((score - 0.5).abs() < 0.001, "unexpected score {score}");
    }
}
