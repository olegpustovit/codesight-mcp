# SENTRUX_IDEA_STEALING.md

Specification for integrating the best ideas from
[sentrux](https://github.com/sentrux/sentrux) into codesight-mcp.
Sentrux is a structural quality sensor for AI-assisted coding; we are a
retrieval engine. They measure architecture; we navigate it. The overlap
is narrow, but three of their ideas close real gaps in our product.

> **Design principle:** every new metric must be computable from data
> already in our SQLite index (dep edges, symbols, complexity, churn).
> No new parsing passes. No new runtime dependencies. The only new
> storage is a single `quality_snapshots` table for the regression gate.

---

## 1. Composite Quality Signal

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
| **Modularity (Q)** | Newman's Q (2004) — edge clustering quality | Community detection on dep graph | `IndexStore::dependency_graph_strict()` |
| **Acyclicity** | Tarjan SCC count | `1 / (1 + cycle_count)` | `find_cycles()` — already computed in `repo_health` |
| **Depth** | Lakos levelization | `1 / (1 + max_depth / 8)` | BFS on dep graph (new, ~20 LoC) |
| **Equality** | Gini coefficient | `1 - gini(per_fn_complexity)` | `all_symbols_for_analysis()` + `cyclomatic_complexity()` |
| **Redundancy** | Dead + duplicate ratio | `1 - (dead_files + dup_fns) / total` | `dead_code_payload()` + body-hash grouping |

**Composite signal** = `(Q * A * D * E * R).powf(1.0/5.0) * 10000`

Integer 0-10000. Displayed in `repo_health` findings and available
standalone via `analysis=quality_signal`.

### Implementation: Modularity Q (Newman 2004)

Newman's modularity measures how well a graph clusters into modules
compared to a random graph with the same degree distribution.

```
Q = (1/2m) * sum_ij [ A_ij - (k_i * k_j) / 2m ] * delta(c_i, c_j)
```

Where:
- `A_ij` = adjacency (1 if edge exists, 0 otherwise)
- `k_i` = degree of node i
- `m` = total edge count
- `c_i` = community assignment of node i
- `delta` = 1 if same community, 0 otherwise

**Community detection**: We use directory-based assignment (each
top-level directory = one community). This is fast, deterministic, and
matches how Rust workspaces and most codebases are actually structured.
No Louvain/Leiden needed — the filesystem IS the module hierarchy.

```rust
// src/analysis/graph/modularity.rs (~60 LoC)

/// Assign each file to a community based on its top-level directory.
fn directory_communities(files: &[String]) -> HashMap<String, usize> {
    let mut dir_to_id: HashMap<String, usize> = HashMap::new();
    let mut assignments: HashMap<String, usize> = HashMap::new();
    for file in files {
        let dir = file.split('/').next().unwrap_or(".");
        let id = {
            let next = dir_to_id.len();
            *dir_to_id.entry(dir.to_string()).or_insert(next)
        };
        assignments.insert(file.clone(), id);
    }
    assignments
}

/// Newman's modularity Q on a file dependency graph.
///
/// Communities are top-level directories. Returns Q in [-0.5, 1.0].
/// Higher = edges cluster within modules. A random graph gives Q ≈ 0.
pub fn compute_modularity_q(
    adj: &HashMap<String, Vec<String>>,
) -> f64 {
    let files: Vec<String> = adj.keys().cloned().collect();
    let communities = directory_communities(&files);

    let mut m: usize = 0; // total edges
    let mut degrees: HashMap<&str, usize> = HashMap::new();

    for (src, targets) in adj {
        for tgt in targets {
            if tgt != src {
                m += 1;
                *degrees.entry(src.as_str()).or_default() += 1;
                *degrees.entry(tgt.as_str()).or_default() += 1;
            }
        }
    }

    if m == 0 {
        return 0.0; // no edges, undefined
    }

    let two_m = (2 * m) as f64;
    let mut q_sum = 0.0;

    for (src, targets) in adj {
        let Some(&c_src) = communities.get(src) else { continue };
        let k_src = *degrees.get(src.as_str()).unwrap_or(&0) as f64;
        for tgt in targets {
            if tgt == src { continue; }
            let Some(&c_tgt) = communities.get(tgt) else { continue };
            if c_src != c_tgt { continue; } // delta = 0
            let k_tgt = *degrees.get(tgt.as_str()).unwrap_or(&0) as f64;
            q_sum += 1.0 - (k_src * k_tgt) / two_m;
        }
    }

    q_sum / two_m
}
```

**Normalization**: `modularity_score = ((Q + 0.5) / 1.5).clamp(0.0, 1.0)`

Maps Q range [-0.5, 1.0] to [0.0, 1.0]. A well-modularized codebase
(Q > 0.3) scores > 0.53. A randomly connected mess (Q ≈ 0) scores 0.33.

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
// src/analysis/graph/depth.rs (~30 LoC)

/// Compute maximum dependency depth via iterative BFS levelization.
/// Returns 0 for an empty or acyclic-root graph.
pub fn max_dependency_depth(
    adj: &HashMap<String, Vec<String>>,
) -> usize {
    // Find roots: nodes with zero in-degree
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    for targets in adj.values() {
        for tgt in targets {
            *in_degree.entry(tgt.as_str()).or_default() += 1;
        }
    }

    let mut queue: VecDeque<&str> = adj.keys()
        .filter(|k| !in_degree.contains_key(k.as_str()))
        .map(|k| k.as_str())
        .collect();

    let mut depth: HashMap<&str, usize> = HashMap::new();
    for &root in &queue {
        depth.insert(root, 0);
    }

    let mut max_depth = 0usize;
    while let Some(node) = queue.pop_front() {
        let d = depth[node];
        if let Some(targets) = adj.get(node) {
            for tgt in targets {
                let entry = depth.entry(tgt.as_str()).or_insert(0);
                if d + 1 > *entry {
                    *entry = d + 1;
                    max_depth = max_depth.max(d + 1);
                    queue.push_back(tgt.as_str());
                }
            }
        }
    }

    max_depth
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

/// Count functions with identical body hashes (after whitespace normalization).
pub fn duplicate_function_count(store: &IndexStore) -> anyhow::Result<usize> {
    let symbols = store.all_symbols_for_analysis()?;
    let mut body_hashes: HashMap<u64, usize> = HashMap::new();
    for sym in &symbols {
        if sym.kind != "function" && sym.kind != "method" { continue; }
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

**Score**: `redundancy_score = (1.0 - (dead + dups) as f64 / total as f64).clamp(0.0, 1.0)`

### Aggregation: The Root Cause Score

```rust
// src/analysis/quality_signal.rs (~80 LoC)

pub struct RootCauseScores {
    pub modularity: f64,
    pub acyclicity: f64,
    pub depth: f64,
    pub equality: f64,
    pub redundancy: f64,
}

pub struct QualitySignal {
    pub score: u32,           // 0-10000
    pub root_causes: RootCauseScores,
    pub assessment: &'static str,
}

pub fn compute_quality_signal(store: &IndexStore) -> anyhow::Result<QualitySignal> {
    let adj = store.dependency_graph_strict()?;
    let cycles = find_cycles(&adj);
    let all_symbols = store.all_symbols_for_analysis()?;

    // 1. Modularity Q
    let q = compute_modularity_q(&adj);
    let modularity = ((q + 0.5) / 1.5).clamp(0.0, 1.0);

    // 2. Acyclicity
    let acyclicity = 1.0 / (1.0 + cycles.len() as f64);

    // 3. Depth
    let max_depth = max_dependency_depth(&adj);
    let depth = 1.0 / (1.0 + max_depth as f64 / 8.0);

    // 4. Equality (Gini over per-fn complexity)
    let complexities: Vec<f64> = all_symbols.iter()
        .filter(|s| s.kind == "function" || s.kind == "method")
        .map(|s| cyclomatic_complexity(&s.source_text) as f64)
        .collect();
    let equality = (1.0 - gini_coefficient(&complexities)).clamp(0.0, 1.0);

    // 5. Redundancy
    let total = all_symbols.len().max(1);
    let dead = dead_file_count(store)?;
    let dups = duplicate_function_count(store)?;
    let redundancy = (1.0 - (dead + dups) as f64 / total as f64).clamp(0.0, 1.0);

    // Geometric mean (floor at 0.01 to avoid zeroing out)
    let values = [modularity, acyclicity, depth, equality, redundancy];
    let product: f64 = values.iter().map(|v| v.max(0.01)).product();
    let signal = product.powf(1.0 / 5.0);
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
    "modularity": 0.68,    // Newman's Q normalized
    "acyclicity": 0.14,    // 1/(1+6) = 0.14 for 6 cycles
    "depth": 0.50,         // 1/(1+8/8) for depth 8
    "equality": 0.82,      // 1 - Gini
    "redundancy": 0.95     // 1 - dead/total
  },
  "bottleneck": "acyclicity",  // lowest root cause
  "suggestion": "Break 6 dependency cycles to improve score"
}
```

### Performance Target

Must stay under 200ms on this repo (406 files, 4042 symbols). The
heaviest operation is `all_symbols_for_analysis()` + complexity loop —
same work `hotspots` already does. Cache the dep graph across root-cause
computations within a single call.

---

## 2. Session Regression Gate

### Problem

An agent writes code, commits, and moves on — never checking whether
its changes degraded the codebase structure. There is no feedback loop.
Sentrux's `session_start` / `session_end` pattern solves this: snapshot
metrics before, rescan after, diff, flag regressions.

### Design

Two new operations. We do NOT add new tools — these go on existing
surfaces per policy #2 in AGENTS.md ("extensibility goes through
view/analysis enums — never parallel tool names").

#### 2a. `manage_indexes action=snapshot_quality`

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
    avg_complexity REAL NOT NULL,
    PRIMARY KEY (name)
);
```

Only one row per name — `INSERT OR REPLACE`. Snapshots are cheap (one
row) and self-cleaning (overwritten on next snapshot with same name).

**Response:**
```jsonc
{
  "action": "snapshot_quality",
  "snapshot_name": "session",
  "score": 7234,
  "root_causes": { "modularity": 0.68, "acyclicity": 0.14, ... },
  "stored": true
}
```

#### 2b. `evaluate_quality analysis=regression_check`

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
    "snapshot_name": "session",
    "created_at": "2026-04-12T11:00:00Z",
    "score": 7234
  },
  "current": {
    "score": 6891
  },
  "delta": {
    "score": -343,
    "modularity": -0.02,
    "acyclicity": 0.0,
    "depth": 0.0,
    "equality": -0.05,
    "redundancy": -0.01
  },
  "degraded": true,
  "degraded_dimensions": ["equality", "modularity", "redundancy"],
  "improved_dimensions": [],
  "suggestion": "Complexity inequality increased (Gini +0.05). Check new functions — 2 have cyclomatic > 25.",
  "findings": [
    {
      "finding_id": "regression:equality",
      "severity": "warning",
      "summary": "equality dropped 0.82 → 0.77 (Gini coefficient increased)",
      "evidence": { "before": 0.82, "after": 0.77, "delta": -0.05 }
    }
  ]
}
```

**Degradation logic:**
- `degraded = true` if composite score dropped by > 100 points (1%)
  OR any single root cause dropped by > 0.05
- Each degraded dimension gets a finding with severity:
  - `warning` if delta < -0.05
  - `error` if delta < -0.15
- Suggestion text is auto-generated from the worst-degraded dimension

**When no snapshot exists:** returns an error with `code: "NO_BASELINE"`
and a message suggesting `snapshot_quality` first. This is NOT a silent
no-op — the agent must learn to snapshot before starting work.

### Implementation Plan

```
src/analysis/quality_signal.rs     — NEW: ~120 LoC
  compute_quality_signal()
  compute_regression()
  QualitySignal, RootCauseScores, RegressionReport structs

src/analysis/graph/modularity.rs   — NEW: ~60 LoC
  compute_modularity_q()
  directory_communities()

src/analysis/graph/equality.rs     — NEW: ~25 LoC
  gini_coefficient()

src/analysis/graph/depth.rs        — NEW: ~30 LoC
  max_dependency_depth()

src/analysis/graph/mod.rs          — EDIT: add `pub mod modularity, equality, depth;`

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

**Total new code: ~445 LoC**
**Total edits to existing files: ~50 LoC** (enum arms, mod declarations,
migration, tools.json schema)

---

## 3. Test Gap Analysis

### Problem

We know which files are complex (hotspots) and which are central
(PageRank). We don't cross-reference this with test coverage to identify
**high-risk untested code** — the files most likely to break silently.

Sentrux does this by ranking untested source files by
`complexity × fan_in`. We can do better because we have symbol-level
granularity and git churn data.

### Design

New `analysis=test_gaps` on `evaluate_quality`.

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

## 4. Existing `repo_health` Enhancement

Once quality_signal is implemented, we embed it in `repo_health` for
free — it becomes the first finding:

```jsonc
{
  "finding_id": "repo_health:quality_signal",
  "analysis": "repo_health",
  "severity": "info",
  "confidence": 1.0,
  "summary": "Quality signal: 7234/10000 (very_good). Bottleneck: acyclicity (6 cycles).",
  "evidence": {
    "score": 7234,
    "root_causes": { ... },
    "bottleneck": "acyclicity"
  }
}
```

This costs ~5 LoC in `repo_health.rs` — we call `compute_quality_signal()`
which reuses the dep graph and cycle data already loaded in that handler.

---

## 5. What We Explicitly Do NOT Take

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

### Phase 1: Quality Signal (P0)

1. `src/analysis/graph/modularity.rs` — Newman's Q
2. `src/analysis/graph/equality.rs` — Gini coefficient
3. `src/analysis/graph/depth.rs` — Lakos depth
4. `src/analysis/quality_signal.rs` — aggregation + duplicate detection
5. Handler arm in `quality/mod.rs`
6. `tools.json` schema update
7. Integration test
8. Embed in `repo_health` response

**Estimated: ~300 LoC new, ~20 LoC edits**

### Phase 2: Session Regression Gate (P0)

1. SQLite migration for `quality_snapshots`
2. `crates/codesight-storage/src/storage/snapshot_store.rs`
3. `snapshot_quality` action in `index_admin.rs`
4. `regression_check` analysis in quality handler
5. `tools.json` schema update
6. Integration tests

**Estimated: ~210 LoC new, ~30 LoC edits**

### Phase 3: Test Gaps (P1)

1. `src/analysis/test_gaps.rs`
2. Handler arm in `quality_more.rs`
3. `tools.json` schema update
4. Integration test

**Estimated: ~140 LoC new, ~10 LoC edits**

### Total Budget

| Phase | New LoC | Edit LoC | New Files | Edited Files |
|---|---|---|---|---|
| Quality Signal | ~300 | ~20 | 4 | 4 |
| Regression Gate | ~210 | ~30 | 2 | 4 |
| Test Gaps | ~140 | ~10 | 1 | 3 |
| **Total** | **~650** | **~60** | **7** | **~8** |

This is a net-positive trade if we simultaneously apply the `simplify`
skill to eliminate ≥ 650 LoC of existing bloat in the same sessions.
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
