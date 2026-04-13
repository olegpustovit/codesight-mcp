# SENTRUX_IDEA_STEALING.md

Specification for integrating the best ideas from
[sentrux](https://github.com/sentrux/sentrux) into codesight-mcp.
Sentrux is a structural quality sensor for AI-assisted coding; we are a
retrieval engine. They measure architecture; we navigate it. The overlap
is narrow, but three of their ideas close real gaps in our product.

> **Design principle:** every new metric must be computable from data
> already in our SQLite index (dep edges, symbols, complexity, churn).
> No new parsing passes. No new runtime dependencies. The only durable
> storage added by this plan is a small `quality_snapshots` table for the
> regression gate.

> **Reality check:** this plan improves quality telemetry; it does **not**
> by itself make codesight "work on any codebase". That broader promise
> still depends on extractor coverage, adaptive-language wiring during
> ingest, and graceful degradation when strict dependency edges are sparse.
> Every metric below therefore carries an explicit coverage/confidence
> contract. If the index is partial, the score must degrade to
> `"confidence": "low"` instead of pretending to be precise.

## 0. Preconditions For The "Works Everywhere" Claim

These are outside the Sentrux feature steal itself, but they are required
if we want to claim that the MCP server runs without hand configuration
and produces trustworthy metrics on arbitrary repos.

1. **Zero-config startup must stay true.** Today this is already mostly
   satisfied:
   - `codesight-mcp check` works with no config file.
   - `AppConfig::load()` falls back to a writable default index root.
   - `CodeService::new()` creates storage layout eagerly.
   This plan must not introduce any mandatory config knobs.
2. **Adaptive-language flow must be wired into ingest, not just exist as
   helper code.** The repo already has `src/adaptive_languages.rs`, but
   the ingest path must actually update `.codesight.toml` / active
   language selection before we can claim "adapts to any codebase".
3. **Every new metric must tolerate partial graphs.**
   - If dependency coverage is weak, graph-derived dimensions are marked
     `unavailable` or low-confidence.
   - Composite scoring uses only available dimensions.
   - Regression gating compares only dimensions that are available in both
     baseline and current snapshots.
4. **The response must say when it is guessing.** If a score depends on
   heuristics such as name matching, line-span proxies, or missing git
   history, the MCP response includes `coverage` and `confidence`, and
   `repo_health` bubbles that up in its findings.

## 0b. Quality Strategy: Match Sentrux, Then Beat It

If we ship only one structural score, we will at best match Sentrux's
story and likely lose on clarity. codesight should instead ship a
**dual-layer metric model**:

1. **Structural Core**: Sentrux-parity architecture score.
   This answers "is the codebase structurally healthy?"
2. **Agent Readiness**: codesight-native score for retrieval, navigation,
   and safe change-making.
   This answers "will an agent actually find the right thing and edit it
   safely?"
3. **Governance Score**: geometric mean of Structural Core and Agent
   Readiness.
   This is the top-line score used for session gating.

That gives us a stronger product position:

- **At least as good as Sentrux** on architecture because the structural
  core stands on its own.
- **Strictly better than Sentrux** for agent workflows because we measure
  failure modes they do not: ambiguity, missing test backstops, missing
  docs/context, and change surface volatility.

---

## 1. Structural Core Signal

### Problem

We report individual metrics (cyclomatic complexity, cycle count, dead
code %, hotspot scores) but never combine them into a single number.
An agent has no way to answer "did my change make the codebase
objectively better or worse?" without interpreting 5+ disconnected
values.

Sentrux solved this with a 0-10000 composite score from 5 root-cause
metrics aggregated via geometric mean. The geometric mean is
Nash-optimal: you cannot game one dimension while tanking another.

### Design

**Five root-cause metrics**, each normalized to [0.0, 1.0]:

| Metric | Theory | What we compute | Data source |
|---|---|---|---|
| **Module cohesion** | Keep imports inside coherent boundaries | share of strict import edges that stay within the same top-level module | `IndexStore::dependency_graph_strict()` |
| **Acyclicity** | Tarjan SCC count | `1 / (1 + cycle_count)` | `find_cycles()` — already computed in `repo_health` |
| **Depth** | Lakos-style layering | `1 / (1 + condensed_dag_depth / 8)` | SCC-condensed DAG from strict dep graph |
| **Equality** | Gini coefficient | `1 - gini(per_fn_complexity)` | `all_symbols_for_analysis()` + `cyclomatic_complexity()` |
| **Redundancy** | Dead + duplicate ratio | `1 - weighted(dead_file_ratio, dup_fn_ratio)` | `dead_code_payload()` + body-hash grouping |

**Structural Core** = geometric mean of the **available** root-cause
dimensions, multiplied by 10000.

Integer 0-10000. Displayed in `repo_health` findings and available
standalone via `analysis=quality_signal`.

**Availability rule:** at least 3 dimensions must be available, and at
least 2 of those must come from structural data rather than pure
complexity heuristics. Otherwise the tool returns a low-confidence result
and does not participate in regression gating.

### Implementation: Module Cohesion

The earlier draft used Newman's modularity `Q`. That is a poor fit here:
the graph is directed, sparse, and community assignment is fixed by the
filesystem rather than detected by an optimizer. A half-correct
modularity implementation is worse than a simpler metric that is
obviously right.

Use a deterministic boundary-cohesion score instead:

- each indexed file is assigned to a top-level module
  (`src`, `crates`, `tests`, `.`).
- count strict dependency edges `src -> tgt`.
- compute:

```
cohesion = internal_edges / total_edges
```

- normalize with a floor so repos with unavoidable cross-boundary imports
  are not unfairly punished:

```
module_cohesion_score = ((cohesion - 0.2) / 0.8).clamp(0.0, 1.0)
```

This is faster, deterministic, and easy to explain in the MCP response.
It also lets us surface raw evidence directly:
`internal_edges`, `cross_edges`, `largest_cross_edge_pairs`.

```rust
// src/analysis/quality_signal.rs

fn top_level_module(path: &str) -> &str {
    path.split('/').next().unwrap_or(".")
}

fn compute_module_cohesion(
    adj: &HashMap<String, Vec<String>>,
) -> Option<f64> {
    let mut total = 0usize;
    let mut internal = 0usize;
    for (src, targets) in adj {
        for tgt in targets {
            if src == tgt {
                continue;
            }
            total += 1;
            if top_level_module(src) == top_level_module(tgt) {
                internal += 1;
            }
        }
    }
    (total > 0).then_some(internal as f64 / total as f64)
}
```

If `adj` has zero usable edges, the dimension is unavailable rather than
pretending the repo is perfectly modular.

### Implementation: Gini Coefficient

Measures inequality in per-function complexity. A codebase where all
functions have similar complexity scores high (equal distribution). A
codebase with 3 god-functions and 200 trivial ones scores low.

```rust
// src/analysis/graph/equality.rs (~25 LoC)

/// Gini coefficient over a distribution of values.
/// Returns 0.0 (perfect equality) to 1.0 (maximum inequality).
pub fn gini_coefficient(values: &[f64]) -> f64 {
    let n = values.len();
    if n == 0 { return 0.0; }

    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let sum: f64 = sorted.iter().sum();
    if sum == 0.0 { return 0.0; }

    let mut cumulative = 0.0;
    let mut area_under_lorenz = 0.0;
    for &v in &sorted {
        cumulative += v;
        area_under_lorenz += cumulative;
    }

    // Gini = 1 - 2 * (area under Lorenz) / (n * sum) + 1/n
    // Simplified from the standard formula.
    let gini = 1.0 - (2.0 * area_under_lorenz) / (n as f64 * sum) + 1.0 / n as f64;
    gini.clamp(0.0, 1.0)
}
```

**Score**: `equality_score = 1.0 - gini(per_fn_complexities)`

### Implementation: Lakos Depth

Longest dependency chain from any root to any leaf. Deep dependency
trees are fragile (changes cascade through many layers).

```rust
// src/analysis/quality_signal.rs

/// Compute layering depth on the SCC-condensed DAG.
/// Raw BFS on the original graph is wrong when cycles exist because it can
/// revisit nodes indefinitely or exaggerate depth inside a single cycle.
pub fn condensed_dependency_depth(
    adj: &HashMap<String, Vec<String>>,
) -> Option<usize> {
    let sccs = tarjan_scc(adj);
    let dag = condense_to_dag(adj, &sccs);
    longest_path_in_dag(&dag)
}
```

**Score**: `depth_score = 1.0 / (1.0 + max_depth as f64 / 8.0)`

Depth 0 = 1.0 (flat). Depth 8 = 0.5 (moderate). Depth 24 = 0.25 (deep).

### Implementation: Redundancy

Ratio of dead files + duplicate function bodies to total. Dead code we
already have. Duplicate detection uses body hashing (already stored in
`symbols.source_text`).

```rust
// Extend existing dead_code module (~15 LoC for dup detection)

/// Count functions with identical body hashes (after lightweight normalization).
pub fn duplicate_function_count(store: &IndexStore) -> anyhow::Result<usize> {
    let symbols = store.all_symbols_for_analysis()?;
    let mut body_hashes: HashMap<u64, usize> = HashMap::new();
    for sym in &symbols {
        if sym.kind != "function" && sym.kind != "method" { continue; }
        if sym.source_text.trim().len() < 40 { continue; } // ignore trivial bodies
        let hash = hash_normalized_body(&sym.source_text);
        *body_hashes.entry(hash).or_default() += 1;
    }
    // Count symbols that are duplicates (hash appears 2+ times)
    Ok(body_hashes.values().filter(|&&c| c >= 2).map(|c| c - 1).sum())
}

fn hash_normalized_body(source: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // Normalize: collapse whitespace, strip comments
    let normalized: String = source
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    normalized.hash(&mut hasher);
    hasher.finish()
}
```

**Score** must use consistent denominators:

```rust
dead_ratio = dead_files as f64 / total_files.max(1) as f64;
dup_ratio  = duplicate_functions as f64 / total_functions.max(1) as f64;
redundancy_score = 1.0 - (0.6 * dead_ratio + 0.4 * dup_ratio);
```

Mixing `dead_files` and `dup_fns` over `all_symbols.len()` would distort
the metric and make changes in one dimension swamp the other.

### Aggregation: The Root Cause Score

```rust
// src/analysis/quality_signal.rs (~80 LoC)

pub struct RootCauseScores {
    pub modularity: Option<f64>,
    pub acyclicity: f64,
    pub depth: Option<f64>,
    pub equality: f64,
    pub redundancy: f64,
}

pub struct QualitySignal {
    pub score: u32,           // 0-10000
    pub root_causes: RootCauseScores,
    pub assessment: &'static str,
    pub confidence: &'static str,
}

pub fn compute_quality_signal(store: &IndexStore) -> anyhow::Result<QualitySignal> {
    let adj = store.dependency_graph_strict()?;
    let cycles = find_cycles(&adj);
    let all_symbols = store.all_symbols_for_analysis()?;

    // 1. Module cohesion
    let modularity = compute_module_cohesion(&adj)
        .map(|cohesion| ((cohesion - 0.2) / 0.8).clamp(0.0, 1.0));

    // 2. Acyclicity
    let acyclicity = 1.0 / (1.0 + cycles.len() as f64);

    // 3. Depth
    let depth = condensed_dependency_depth(&adj)
        .map(|max_depth| 1.0 / (1.0 + max_depth as f64 / 8.0));

    // 4. Equality (Gini over per-fn complexity)
    let complexities: Vec<f64> = all_symbols.iter()
        .filter(|s| s.kind == "function" || s.kind == "method")
        .map(|s| cyclomatic_complexity(&s.source_text) as f64)
        .collect();
    let equality = (1.0 - gini_coefficient(&complexities)).clamp(0.0, 1.0);

    // 5. Redundancy
    let total_files = store.file_count()?.max(1);
    let total_functions = complexities.len().max(1);
    let dead = dead_file_count(store)?;
    let dups = duplicate_function_count(store)?;
    let redundancy = (
        1.0 - (
            0.6 * (dead as f64 / total_files as f64) +
            0.4 * (dups as f64 / total_functions as f64)
        )
    ).clamp(0.0, 1.0);

    // Geometric mean over available dimensions only.
    let values: Vec<f64> = [modularity, Some(acyclicity), depth, Some(equality), Some(redundancy)]
        .into_iter()
        .flatten()
        .collect();
    if values.len() < 3 {
        return Ok(QualitySignal {
            score: 0,
            root_causes: RootCauseScores {
                modularity,
                acyclicity,
                depth,
                equality,
                redundancy,
            },
            assessment: "insufficient_data",
            confidence: "low",
        });
    }
    let confidence = if values.len() >= 5 { "high" } else { "medium" };
    let product: f64 = values.iter().map(|v| v.max(0.01)).product();
    let signal = product.powf(1.0 / values.len() as f64);
    let score = (signal * 10000.0).round() as u32;

    let assessment = match score {
        0..=3000 => "poor",
        3001..=5000 => "fair",
        5001..=7000 => "good",
        7001..=8500 => "very_good",
        _ => "excellent",
    };

    Ok(QualitySignal {
        score,
        root_causes: RootCauseScores {
            modularity,
            acyclicity,
            depth,
            equality,
            redundancy,
        },
        assessment,
        confidence,
    })
}
```

### MCP Surface

New `analysis` variant on `evaluate_quality`:

```jsonc
// tools.json addition to evaluate_quality.analysis enum:
// "quality_signal"

// Response shape:
{
  "analysis": "quality_signal",
  "score": 7234,           // 0-10000
  "assessment": "very_good",
    "root_causes": {
      "modularity": 0.68,    // module cohesion normalized
      "acyclicity": 0.14,    // 1/(1+6) = 0.14 for 6 cycles
      "depth": 0.50,         // 1/(1+8/8) for depth 8
      "equality": 0.82,      // 1 - Gini
      "redundancy": 0.95     // 1 - dead/total
  },
  "coverage": {
    "graph_files": 381,
    "graph_edges": 942,
    "functions_with_source": 1604,
    "dimensions_available": 5
  },
  "confidence": "high",
  "bottleneck": "acyclicity",  // lowest root cause
  "suggestion": "Break 6 dependency cycles to improve score"
}
```

### Performance Target

Must stay under 200ms on this repo (406 files, 4042 symbols). The
heaviest operation is `all_symbols_for_analysis()` + complexity loop —
same work `hotspots` already does. Cache the dep graph across root-cause
computations within a single call.

### Why This Only Gets Us To Parity

Sentrux explicitly optimizes for architectural drift: five structural
dimensions, one quality signal, one session gate. Their public docs
center on exactly that model, including the five root causes, the
single-number signal, and session baseline/diff workflow.[^sentrux_site]
If we stop here, we match the category but do not surpass it.

---

## 2. Agent Readiness Signal

### Problem

codesight is not just an architecture sensor; it is a retrieval engine
for coding agents. A codebase can be structurally decent yet still be a
bad substrate for agentic work:

- too many same-name symbols across unrelated modules
- important code paths have no tests near them
- central APIs have no docs or local context
- small edits fan out through hot, high-centrality files

These are not secondary concerns for us. They are the core product
failure modes.

### Design

**Four agent-facing dimensions**, each normalized to [0.0, 1.0]:

| Metric | Question | What we compute | Data source |
|---|---|---|---|
| **Discoverability** | Will search land on the right symbol fast? | weighted inverse of same-name ambiguity among important symbols | `all_symbols_for_analysis()` + in-degree/churn |
| **Test backstop** | If the agent edits here, how likely is breakage caught? | covered high-risk mass / total high-risk mass | `test_gaps` ingredients |
| **Context coverage** | Will the agent find docs for important APIs? | documented importance mass / total importance mass | doc index + centrality/fan-in |
| **Change locality** | Do edits stay local or propagate explosively? | inverse weighted blast radius of hot/central files | dep graph + churn |

#### 2a. Discoverability

The biggest agent failure mode is not "high complexity" in isolation.
It is landing on the wrong similarly-named thing.

We score discoverability using **weighted name ambiguity**:

```rust
importance(sym) = 1.0
    + ln(1 + fan_in(sym.file))
    + ln(1 + churn(sym.file));

ambiguity(sym) = max(0, variants(sym.normalized_name) - 1);

discoverability_score =
    1.0 / (1.0 + weighted_mean(ambiguity, importance));
```

Rules:

- only count cross-file variants
- discount variants inside the same module tree
- ignore tiny helper names only if their importance is near zero
- surface the worst collisions in the response so the score is actionable

This is a better agent metric than raw modularity because it measures the
probability of retrieving the wrong target.

#### 2b. Test Backstop

Re-use the planned `test_gaps` risk model, but invert it into a score:

```rust
risk(sym) = cyclomatic * (1 + fan_in) * (1 + ln(1 + churn));

test_backstop_score =
    covered_high_risk_mass / total_high_risk_mass.max(epsilon);
```

This is better than file-level coverage percentage because it weights the
code the agent is most likely to damage.

#### 2c. Context Coverage

This is the doc equivalent of risk-weighted test coverage:

```rust
context_coverage_score =
    documented_importance_mass / total_importance_mass.max(epsilon);
```

Where `documented` means:

- exact symbol-name heading match in the doc index, or
- strong section-body mention with high confidence

Only important symbols count materially. A repo can have thousands of
undocumented private helpers and still be agent-usable if its key entry
points are documented.

#### 2d. Change Locality

This measures how much a likely edit fans out:

```rust
surface(file) = (1 + importers(file)) * (1 + ln(1 + churn(file)));

change_locality_score =
    1.0 / (1.0 + weighted_hotspot_surface / 8.0);
```

Where `weighted_hotspot_surface` is computed over the risky slice of the
repo, not every file equally:

- top hotspot files
- files participating in cycles
- files with high importer count

This makes the metric more sensitive to where agents actually operate.

### Aggregation

**Agent Readiness** = geometric mean of the available agent-facing
dimensions, multiplied by 10000.

This score is separate from Structural Core on purpose. A repo might have:

- good structure, bad agent readiness
- bad structure, decent local docs/tests
- both good
- both bad

That distinction is valuable; collapsing it too early throws signal away.

### MCP Surface

`evaluate_quality analysis=quality_signal` should return all three
layers:

```jsonc
{
  "analysis": "quality_signal",
  "score": 7519,                    // governance_score
  "structural_score": 7234,
  "agent_readiness_score": 7810,
  "assessment": "very_good",
  "structural_root_causes": {
    "modularity": 0.68,
    "acyclicity": 0.14,
    "depth": 0.50,
    "equality": 0.82,
    "redundancy": 0.95
  },
  "agent_root_causes": {
    "discoverability": 0.73,
    "test_backstop": 0.64,
    "context_coverage": 0.58,
    "change_locality": 0.87
  },
  "coverage": {
    "graph_files": 381,
    "graph_edges": 942,
    "functions_with_source": 1604,
    "dimensions_available": 9
  },
  "confidence": "high",
  "bottleneck": "context_coverage",
  "bottleneck_layer": "agent_readiness",
  "suggestion": "Document the top fan-in public APIs; they are central but weakly covered in docs."
}
```

**Governance Score** is:

```rust
governance_score = sqrt(structural_core * agent_readiness) * 10000
```

This should be the top-line value in `score`, because it is the number
that best answers "will future agent sessions go well?".

---

## 3. Session Regression Gate

### Problem

An agent writes code, commits, and moves on — never checking whether
its changes degraded the codebase structure. There is no feedback loop.
Sentrux's `session_start` / `session_end` pattern solves this: snapshot
metrics before, rescan after, diff, flag regressions.

### Design

Two new operations. We do NOT add new tools — these go on existing
surfaces per policy #2 in AGENTS.md ("extensibility goes through
view/analysis enums — never parallel tool names").

The gate should compare **both** layers:

- Structural Core regression: architectural degradation
- Agent Readiness regression: retrieval/editability degradation
- Governance Score regression: top-line pass/fail

Hard rule:

- if Structural Core drops materially, fail the gate
- if Agent Readiness drops materially, fail or warn depending on
  confidence and severity
- if Governance Score drops but both layers are statistically flat,
  treat as noise and report it as such

#### 3a. `manage_indexes action=snapshot_quality`

Saves current quality metrics as a named baseline in SQLite.

**Request:**
```jsonc
{
  "name": "manage_indexes",
  "arguments": {
    "source_id": "OlegHQ/codesight-mcp",
    "action": "snapshot_quality",
    "snapshot_name": "pre-refactor"  // optional, defaults to "session"
  }
}
```

**What gets stored** (new table `quality_snapshots`):

```sql
CREATE TABLE IF NOT EXISTS quality_snapshots (
    source_id   TEXT NOT NULL,
    name        TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    score       INTEGER NOT NULL,       -- 0-10000
    modularity  REAL NOT NULL,
    acyclicity  REAL NOT NULL,
    depth       REAL NOT NULL,
    equality    REAL NOT NULL,
    redundancy  REAL NOT NULL,
    cycle_count INTEGER NOT NULL,
    dead_count  INTEGER NOT NULL,
    dup_count   INTEGER NOT NULL,
    file_count  INTEGER NOT NULL,
    symbol_count INTEGER NOT NULL,
    graph_file_count INTEGER NOT NULL,
    graph_edge_count INTEGER NOT NULL,
    confidence  TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    avg_complexity REAL NOT NULL,
    PRIMARY KEY (source_id, name)
);
```

Only one row per `(source_id, name)` — `INSERT OR REPLACE`. Snapshots are
cheap (one row) and self-cleaning (overwritten on next snapshot with the
same logical name for the same repo).

**Response:**
```jsonc
{
  "action": "snapshot_quality",
  "source_id": "OlegHQ/codesight-mcp",
  "snapshot_name": "session",
  "score": 7234,
  "root_causes": { "modularity": 0.68, "acyclicity": 0.14, ... },
  "confidence": "high",
  "stored": true
}
```

#### 3b. `evaluate_quality analysis=regression_check`

Compares current metrics against a stored snapshot.

**Request:**
```jsonc
{
  "name": "evaluate_quality",
  "arguments": {
    "source_id": "OlegHQ/codesight-mcp",
    "analysis": "regression_check",
    "snapshot_name": "session"  // optional, defaults to "session"
  }
}
```

**Response:**
```jsonc
{
  "analysis": "regression_check",
  "baseline": {
    "source_id": "OlegHQ/codesight-mcp",
    "snapshot_name": "session",
    "created_at": "2026-04-12T11:00:00Z",
    "score": 7519,
    "structural_score": 7234,
    "agent_readiness_score": 7810
  },
  "current": {
    "score": 7160,
    "structural_score": 7198,
    "agent_readiness_score": 7122
  },
  "delta": {
    "score": -359,
    "structural_score": -36,
    "agent_readiness_score": -688,
    "modularity": -0.02,
    "acyclicity": 0.0,
    "depth": 0.0,
    "equality": -0.01,
    "redundancy": -0.01,
    "discoverability": -0.19,
    "test_backstop": -0.04,
    "context_coverage": -0.12,
    "change_locality": -0.02
  },
  "degraded": true,
  "degraded_dimensions": ["discoverability", "context_coverage"],
  "improved_dimensions": [],
  "confidence_changed": false,
  "suggestion": "Agent readiness degraded: same-name ambiguity increased in high-importance symbols. Collapse duplicate service/helper names or qualify module boundaries.",
  "findings": [
    {
      "finding_id": "regression:discoverability",
      "severity": "warning",
      "summary": "discoverability dropped 0.81 → 0.62 (important symbol-name ambiguity increased)",
      "evidence": { "before": 0.81, "after": 0.62, "delta": -0.19 }
    }
  ]
}
```

**Degradation logic:**
- `degraded = true` if governance score dropped by > 100 points (1%)
  OR structural score dropped by > 100 points
  OR any single high-confidence dimension dropped by > 0.05
- Each degraded dimension gets a finding with severity:
  - `warning` if delta < -0.05
  - `error` if delta < -0.15
- Suggestion text is auto-generated from the worst-degraded dimension
- If `confidence` fell or graph coverage changed materially, return a
  warning and suppress hard regression failure unless a non-heuristic
  dimension still clearly degraded.

**When no snapshot exists:** returns an error with `code: "NO_BASELINE"`
and a message suggesting `snapshot_quality` first. This is NOT a silent
no-op — the agent must learn to snapshot before starting work.

### Implementation Plan

```
src/analysis/quality_signal.rs     — NEW: ~120 LoC
  compute_quality_signal()
  compute_regression()
  compute_module_cohesion()
  condensed_dependency_depth()
  QualitySignal, RootCauseScores, RegressionReport structs

crates/codesight-storage/          — EDIT:
  src/storage/migrations.rs        — add quality_snapshots table migration
  src/storage/mod.rs               — add snapshot_store submodule
  src/storage/snapshot_store.rs    — NEW: ~60 LoC
    save_snapshot(), load_snapshot(), delete_snapshot()

src/service/agent_v1/quality/      — EDIT:
  mod.rs                           — add "quality_signal" and "regression_check" arms
  quality_signal.rs                — NEW: ~80 LoC handler
  regression.rs                    — NEW: ~70 LoC handler

src/service/agent_v1/index_admin.rs — EDIT: add "snapshot_quality" arm

src/tools.json                     — EDIT:
  evaluate_quality.analysis enum   += "quality_signal", "regression_check"
  manage_indexes.action enum       += "snapshot_quality"
  new optional param               += "snapshot_name" on both

tests/quality_signal.rs            — NEW: integration test
tests/regression_check.rs          — NEW: integration test
```

**Total new code: ~350-450 LoC**
**Total edits to existing files: ~50 LoC** (enum arms, mod declarations,
migration, tools.json schema)

---

## 4. Test Gap Analysis

### Problem

We know which files are complex (hotspots) and which are central
(PageRank). We don't cross-reference this with test coverage to identify
**high-risk untested code** — the files most likely to break silently.

Sentrux does this by ranking untested source files by
`complexity × fan_in`. We can do better because we have symbol-level
granularity and git churn data.

### Design

New `analysis=test_gaps` on `evaluate_quality`.

This is useful, but it is not part of the critical path for "runs
without config and self-corrects structural regressions". It stays P1
and should not block shipping quality scoring + regression gating.

**Risk score** = `cyclomatic * (1 + fan_in) * (1 + ln(1 + churn))`

Where:
- `cyclomatic` = function-level cyclomatic complexity
- `fan_in` = number of files that import this function's file
- `churn` = git commit count touching this file

**Test detection:**
- Files matching `**/test*`, `**/*_test.*`, `**/*_spec.*`,
  `**/tests/**`, `**/__tests__/**`, `**/spec/**` are test files
- A source file is "tested" if any test file imports it (from dep edges)
- A function is "tested" if a test file references its name

**Confidence caveat:** name-reference matching is heuristic and noisy for
short identifiers. Any hit generated from raw substring search carries
`confidence: "low"` unless backed by a structural reference or import
edge.

```rust
// src/analysis/test_gaps.rs (~100 LoC)

pub struct TestGap {
    pub symbol_id: String,
    pub name: String,
    pub file: String,
    pub line: u32,
    pub risk_score: f64,
    pub cyclomatic: usize,
    pub fan_in: usize,
    pub churn: u64,
    pub reason: String,
}

pub fn test_gap_analysis(
    store: &IndexStore,
    top_n: usize,
) -> anyhow::Result<Vec<TestGap>> {
    let adj = store.dependency_graph_strict()?;
    let test_files = identify_test_files(&adj);
    let tested_files = files_imported_by_tests(&adj, &test_files);
    let all_symbols = store.all_symbols_for_analysis()?;
    let file_churn = compute_file_churn(store)?;

    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    for targets in adj.values() {
        for tgt in targets {
            *in_degree.entry(tgt.as_str()).or_default() += 1;
        }
    }

    let mut gaps: Vec<TestGap> = Vec::new();
    for sym in &all_symbols {
        if sym.kind != "function" && sym.kind != "method" { continue; }
        if test_files.contains(&sym.file_path) { continue; }
        if tested_files.contains(&sym.file_path) { continue; }

        let cc = cyclomatic_complexity(&sym.source_text);
        if cc < 3 { continue; } // skip trivial functions
        let fan_in = in_degree.get(sym.file_path.as_str()).copied().unwrap_or(0);
        let churn = file_churn.get(&sym.file_path).copied().unwrap_or(0);
        let risk = cc as f64 * (1.0 + fan_in as f64) * (1.0 + (1.0 + churn as f64).ln());

        gaps.push(TestGap {
            symbol_id: sym.id.clone(),
            name: sym.name.clone(),
            file: sym.file_path.clone(),
            line: sym.start_line,
            risk_score: risk,
            cyclomatic: cc,
            fan_in,
            churn,
            reason: format!(
                "untested, cc={cc}, fan_in={fan_in}, churn={churn}"
            ),
        });
    }

    gaps.sort_by(|a, b| b.risk_score.partial_cmp(&a.risk_score).unwrap());
    gaps.truncate(top_n);
    Ok(gaps)
}
```

### MCP Surface

```jsonc
// tools.json: add "test_gaps" to evaluate_quality.analysis enum

// Request:
{
  "name": "evaluate_quality",
  "arguments": {
    "source_id": "OlegHQ/codesight-mcp",
    "analysis": "test_gaps",
    "top_n": 15,
    "min_complexity": 5
  }
}

// Response:
{
  "analysis": "test_gaps",
  "total_untested_functions": 142,
  "gaps": [
    {
      "symbol_id": "src/indexing/imports/resolve.rs::resolve_rust_specifier#function",
      "name": "resolve_rust_specifier",
      "file": "src/indexing/imports/resolve.rs",
      "line": 330,
      "risk_score": 287.4,
      "cyclomatic": 18,
      "fan_in": 7,
      "churn": 12,
      "reason": "untested, cc=18, fan_in=7, churn=12"
    }
  ]
}
```

**New code: ~120 LoC** (one file + handler arm + tools.json enum)

---

## 5. Existing `repo_health` Enhancement

Once quality_signal is implemented, we embed the layered scores in `repo_health` for
free — it becomes the first finding:

```jsonc
{
  "finding_id": "repo_health:quality_signal",
  "analysis": "repo_health",
  "severity": "info",
  "confidence": 1.0,
  "summary": "Governance score: 7519/10000. Structural: 7234. Agent readiness: 7810. Bottleneck: context_coverage.",
  "evidence": {
    "score": 7519,
    "structural_score": 7234,
    "agent_readiness_score": 7810,
    "structural_root_causes": { ... },
    "agent_root_causes": { ... },
    "bottleneck": "context_coverage"
  }
}
```

This costs ~10-20 LoC in `repo_health.rs` if the helper accepts optional
precomputed graph / cycle inputs. Do that instead of re-querying the same
data twice.

---

## 6. What We Explicitly Do NOT Take

| Sentrux Feature | Why Not |
|---|---|
| **GUI / treemap** | We're headless MCP. No egui. |
| **Rules engine** (`.sentrux/rules.toml`) | Premature. Our users are agents, not CI. If we need this later it's a separate RFC. |
| **Design Structure Matrix** | Niche, high token cost. Our dep graph tools cover the same ground. |
| **Bus factor** | Interesting but noisy on small repos. Revisit later. |
| **Plugin-based language config** | Our `runtime/queries/<lang>/tags.scm` + ExtractionTier system works. Different trade-off. |
| **Running Sentrux as sidecar MCP** | Adds a binary dep, no persistence, free tier hides details, duplicates graph computation. |

---

## Implementation Order

### Phase 0: Prerequisites For Trustworthy Metrics (P0)

1. Wire adaptive-language update flow into ingest so repo-local
   `.codesight.toml` actually reflects detected languages.
2. Add coverage/confidence fields to the response payload for every new
   metric in this document.
3. Keep the zero-config path green:
   - `codesight-mcp check`
   - `codesight-mcp init --workspace`
   - `codesight-mcp serve`

### Phase 1: Quality Signal (P0)

1. `src/analysis/quality_signal.rs` — aggregation, cohesion, depth,
   Gini, duplicate detection
2. Handler arm in `quality/mod.rs`
3. `tools.json` schema update
4. Integration test
5. Embed in `repo_health` response without duplicate graph reads

**Estimated: ~220-300 LoC new, ~20 LoC edits**

### Phase 2: Agent Readiness Signal (P0)

1. `src/analysis/agent_readiness.rs` — discoverability, test backstop,
   context coverage, change locality
2. Extend `quality_signal` handler to report structural + agent layers +
   governance score
3. `tools.json` schema update
4. Targeted integration tests with ambiguous names / missing docs /
   untested hotspot fixtures

**Estimated: ~220-320 LoC new, ~20 LoC edits**

### Phase 3: Session Regression Gate (P0)

1. SQLite migration for `quality_snapshots`
2. `crates/codesight-storage/src/storage/snapshot_store.rs`
3. `snapshot_quality` action in `index_admin.rs`
4. `regression_check` analysis in quality handler
5. `tools.json` schema update
6. Integration tests

**Estimated: ~180-240 LoC new, ~30 LoC edits**

### Phase 4: Test Gaps (P1)

1. `src/analysis/test_gaps.rs`
2. Handler arm in `quality_more.rs`
3. `tools.json` schema update
4. Integration test

**Estimated: ~140 LoC new, ~10 LoC edits**

### Total Budget

| Phase | New LoC | Edit LoC | New Files | Edited Files |
|---|---|---|---|---|
| Quality Signal | ~220-300 | ~20 | 1 | 4 |
| Agent Readiness | ~220-320 | ~20 | 1 | 4 |
| Regression Gate | ~180-240 | ~30 | 1 | 4 |
| Test Gaps | ~140 | ~10 | 1 | 3 |
| **Total** | **~760-1000** | **~80** | **4** | **~10** |

This is a net-positive trade if we simultaneously apply the `simplify`
skill to eliminate at least the same amount of existing bloat in nearby
sessions.
Track the delta in REFACTOR_WORKLOG.md per policy.

---

## Validation Criteria

Each phase ships when:

1. **Tests pass**: `cargo nextest run --lib` + targeted integration test
2. **No regressions**: `cargo clippy --all-targets -- -D warnings` clean
3. **Performance**: quality_signal < 200ms on this repo (406 files)
4. **Dogfood**: run `evaluate_quality analysis=quality_signal` on this
   repo and verify the score is reasonable. Run `snapshot_quality` +
   make a change + `regression_check` and verify delta detection.
5. **Token efficiency**: response size < 2KB for quality_signal,
   < 3KB for regression_check. These are summary tools, not data dumps.
6. **REFACTOR_WORKLOG.md** updated with new metrics section
7. **Zero-config safety**: `codesight-mcp check` still passes with no
   global config and no pre-existing `.codesight.toml`.
8. **Partial-index honesty**: on a repo with sparse import extraction,
   `quality_signal` emits low confidence rather than false precision.
9. **Sentrux parity**: Structural Core alone is good enough to stand as a
   defensible architecture score.
10. **Codesight advantage**: Agent Readiness catches at least one class of
   degradation that Structural Core would miss.

---

## Agent Workflow (The Payoff)

Once all three phases ship, an agent session looks like this:

```
1. catalog_sources → get source_id
2. manage_indexes action=snapshot_quality → baseline saved (score: 7234)
3. ... agent writes code, refactors, adds features ...
4. ingest_sources incremental=true → reindex changes
5. evaluate_quality analysis=regression_check → "degraded: false, score: 7389 (+155)"
   OR
   → "degraded: true, equality dropped 0.82 → 0.71. Fix: new function
      `process_batch` has cyclomatic 42. Split it."
6. Agent self-corrects before committing.
```

This is the feedback loop that Sentrux identified as missing from
AI-assisted development. We implement it natively, with persistent
storage, incremental indexing, and symbol-level granularity — all
things Sentrux lacks.

---

## References

[^sentrux_site]: Sentrux markets a single geometric-mean quality signal
from five structural root causes and a session baseline/diff loop on its
site and GitHub README. See <https://sentrux.dev/> and
<https://github.com/sentrux/sentrux>.
