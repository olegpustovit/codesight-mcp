# PLAN.md — codesight-mcp dogfooding remediation

Concrete fixes for the 13 issues flagged in the 2026-04-22 dogfood (`session_stats`
+ `analysis_health` at commit `18aab4e`). Ordered by priority; every item names
the real file/line to touch and the smallest change that closes the case.

Design rules for everything below:
- **Shrink or stay flat.** No new state on `SavingsLedger` / `SessionState`
  unless the field replaces or collapses an existing one.
- **No new top-level tool names, no new CLI binaries.** Every fix extends a
  consolidated tool or an existing hook.
- **Prefer a one-line predicate to a new abstraction.** The ledger, the
  reindex-state tracker, and the per-tool map already exist; reuse them.

---

## ✅ P0.2 — Cold-ingest budget (done, 28.6s → 1.55s, 18× on this repo)

Shipped. Retained the change set for history:

- `crates/codesight-engine/src/indexing/code_index.rs` — sequential walk +
  `rayon::par_iter` worker + `FileOutcome::{Unchanged, Touched, Updated}` so
  unchanged files skip the SQL `UPDATE files` churn.
- `crates/codesight-engine/src/indexing/indexer.rs` — `Mutex<Option<…>>` →
  `Arc<LanguageRegistry>` cached once per workspace root.
- `crates/codesight-language-support/src/extract_tree_sitter.rs` — thread-local
  `tree_sitter::Parser` pool keyed by a cheap grammar fingerprint.
- `crates/codesight-language-support/src/scm_engine/mod.rs` — `Mutex<HashMap>`
  → `RwLock<HashMap>` for the per-language extractor cache.
- `crates/codesight-language-support/src/dep_graph.rs` — parallel edge
  extraction + explicit `known_paths` so incremental ingests resolve edges
  from new files to unchanged neighbors.

Budget now: <2s on this repo; AGENTS.md §5 gate passes with headroom.

---

## P0 — correctness / trust

### P0.1 — Don't fire `STALE_DATA_WARNING` on unaffected catalog calls

**Evidence.** `freshness.rs::annotate_catalog_with_in_flight` (line 82) fires
whenever *any* source has a reindex in flight — even if the caller asked
about a completely unrelated source. Result: on a warm server with a
background reindex running on repo A, every `catalog_sources` call for any
other repo gets a `STALE_DATA_WARNING`. ~100 entries in
`session_stats.stale_warnings_fired_with_in_flight_paths` carried empty
`files_in_flight`.

**Fix (one file).** `crates/codesight-engine/src/service/agent_v1/freshness.rs`.
Keep `data.indexing: [...]` (it's the machine-readable signal). Drop the
`warnings[]` push — `indexing` already tells any client what's going on, and
the warning is indistinguishable from genuine staleness. The `annotate()`
per-tool path already guards on *touched-file intersection* and is fine.

**Verification.**
- `tests/agent_v1_staleness.rs` — extend: when no sources are in flight,
  assert zero `STALE_DATA_WARNING`. When repo A is mid-reindex and we call
  `catalog_sources`, assert `data.indexing == ["A"]` and *no* warning.
- Spot-check `session_stats` after 10 calls in a quiet repo: empty list.

### P0.3 — `path_rename` must match grouped `use` imports

**Evidence I found in the code.** `impact.rs::collect_path_replacements`
(line 250) string-matches `from` verbatim, bounded by non-identifier bytes.
`boundary_before/boundary_after` already accept `)`, `,`, `>`, `\n`, `{`,
`;` — the plan's earlier hypothesis that those were missing was wrong.

The *real* miss is grouped `use` imports:

```rust
use codesight_language_support::traits::{Language, Other};
//  ^ "codesight_language_support::traits::Language" never appears verbatim
```

For `from = codesight_language_support::traits::Language`, the byte scan
never finds the string because the `::Language` lives inside the brace
group, separated from the prefix.

**Fix.** `crates/codesight-analysis/src/impact.rs` — add a second pass that,
inside any `use PREFIX::{ ... }` body, expands each brace item to
`PREFIX::ITEM` before matching. Only active when `from` starts with a prefix
that equals the `use` body's `PREFIX::`. Cheap: one line-by-line pass over
lines starting with `use ` or `pub use `. Do not replace the byte scanner —
augment it.

Separately, keep the byte scanner change from the original plan to also
guard `:` boundary on one side but allow `$` / `#` to stay rejected.

**Verification.** New `tests/path_rename.rs` with 15 canonical Rust shapes
(plain `use`, grouped `use`, `pub use`, `as` alias, fully qualified call,
`impl Trait for Type` bound, turbofish, in-doc-comment sanity-no-match). Dry
run on the original failing prefix must return `changes >= 60`.

---

## P1 — efficiency

### P1.1 — `catalog_sources` body-cache keyed by reindex generation

**Evidence.** `catalog_sources` handler (`service/agent_v1/catalog.rs:30`)
walks every code source, opens each store via `ensure_repo_store`, snapshots
through `SourceRegistry`. On a 2-source repo that's ~120 ms; 41 calls
dominated the session with `tokens_saved=0`.

**Fix (one file, no ledger touch).**
`crates/codesight-engine/src/service/mod.rs` — add

```rust
catalog_cell: OnceLock<RwLock<Option<(u64 /* generation */, Value /* body */)>>>,
```

to `CodeService`. Bump the generation from:
- `ingest_sources` success,
- `manage_indexes action ∈ {invalidate, delete, rebuild}`.

`catalog_sources` reads the cell with a read-lock; on hit, returns the
cached body with `_meta.cache_hit = true` and zero SQLite work. On miss,
builds the body, stores it under the generation, returns.

Not on the ledger — this is transient per-server-instance memoisation, not
session telemetry.

**Verification.** New `tests/catalog_sources_cache.rs`: 10 consecutive calls
→ call 1 misses, calls 2..10 return `cache_hit: true` and `timing_ms < 2`.
After `ingest_sources`, next call misses.

### P1.2 — Fuzzy search normalises identifier case before trigrams

**Fix.** `crates/codesight-retrieval/src/fuzzy.rs` + `ngram.rs`. Introduce

```rust
fn normalize_identifier(s: &str) -> String { /* lowercase, drop non-alnum */ }
```

applied once to the query side and once when building the trigram set per
symbol. Raw form stays in the stored row for display. Edit-distance cutoff
operates on the normalised form.

**Verification.** Unit tests in `fuzzy.rs`:
- `resolveImprts` ↔ `resolve_imports_to_files` within threshold.
- `MyStruct` ↔ `my_struct`.
- `foo_bar` ↔ `foo_bar` score == 1.0.

---

## P2 — signal quality

### P2.1 — Retune BM25 so tests/benches don't swamp prod

**Smaller alternative to the original "file_class" column.** BM25 already
has a length prior (`b`, typically 0.75). Keyword-dense test files win
because repeated identifiers inflate term frequency without an offsetting
length penalty. Two single-line knobs fix the symptom with no schema
change:

1. `crates/codesight-retrieval/src/bm25.rs` — raise the default `b` from
   0.75 to ~0.9 for the natural-language path only. Keyword-mode retrieval
   is unchanged (exact-ID lookups should stay crisp).
2. In the same file, add a **directory-prior** multiplier that multiplies
   the final score by 1.0 for everything under `src/` / `crates/*/src/` and
   0.5 for `tests/` / `benches/` / `fixtures/` / `testdata/` / `scripts/`.
   Applied to NL hits only; echo the multiplier in `_meta` under
   `weighting_applied` for observability.

No new column, no new argument, no opt-in flag. If a user genuinely wants
test files, they already narrow via `scope="tests/"`.

**Verification.** Fixture repo with identical keyword density in `src/` and
`tests/`: NL query ranks `src/` first. Re-run the original
"how does import resolution work across languages?" query: top-5 from
`crates/codesight-*/src/`.

### P2.2 — Per-variant error telemetry via key composition

**Fact I verified.** `SessionState.per_tool: HashMap<String, ToolSessionStats>`
already exists. The variant info is already known at dispatch.

**Fix (one line where the ledger key is built).** Compose the existing key
as `format!("{tool}:{variant}")` when a variant is available
(`inspect_graph graph=X`, `inspect_structure view=X`, `evaluate_quality
analysis=X`, `evaluate_impact analysis=X`). `session_stats` renders the
dot-prefixed keys verbatim. No new field, no parallel map, no new
`error_count_by_variant` struct.

**Verification.** Integration test: call `inspect_graph graph=calls
symbol_id=does_not_exist` → `session_stats.per_tool["inspect_graph:calls"]
.error_count == 1`.

### P2.3 — `quality_signal.first_step`: drop when no seed

**Fix.**
`crates/codesight-analysis/src/quality_signal/recommendations.rs` — one
if-let gate per recommendation: if the seed (duplicated name, top-offender
path) is empty, emit `suggestion` without `first_step`. Don't emit a
`first_step` whose `args.query` is `""`.

**Verification.** `tests/quality_signal.rs` asserts: for every returned
recommendation, either `first_step` is absent or its args contain no empty
strings / empty arrays.

---

## P3 — polish

### P3.1 — Uniform float encoding for agent_readiness

**Evidence.**
`crates/codesight-analysis/src/quality_signal/governance.rs:16`
`pub agent_readiness_score: u32` + `/ 10000.0` noise inside
`governance.rs:36`. Elsewhere we already emit floats.

**Fix.** Change the field to `f64` in the `[0.0, 1.0]` range and delete the
`* 10_000` / `/ 10000.0` sites. Keep the JSON key name stable —
`agent_readiness_score` is not harmful; renaming adds churn for zero
semantic value. Update `tests/quality_signal.rs` fixtures.

### P3.2 — `analysis_health.graph.unresolved_use_statements` — surface it

**Fix.** `crates/codesight-engine/src/service/agent_v1/index_admin.rs` —
compute `COUNT(*) FROM dep_edges WHERE fact_quality = 'Heuristic' AND
resolved_target IS NULL` at the same time we build `graph`. Add the field
to the `graph` block. Single SQL, no schema change (column already exists
on dep edges). AGENTS.md claim is now truthful.

**Verification.** Test that seeds a repo with a known-bad import → assert
`graph.unresolved_use_statements > 0`.

### P3.3 — Nudge on repeated full-file reads (reuse existing set)

**Evidence.** `SessionState.codesight_read_paths: BTreeSet<String>` already
exists. What we lack is a *count* per path for the "3rd read" tier.

**Fix.** Replace the `BTreeSet` with `BTreeMap<String, u8>` in
`session.rs` (bump count on every full-file read; skip for
range/symbol/section reads). In `content.rs`:
- 2nd full-file read of a path → attach `SUGGEST_OUTLINE` warning.
- 3rd+ → include a prebuilt `suggested_call` with the right
  `inspect_structure view=file_outline` args.

`edits_after_codesight_read` semantics unchanged — it already uses the
same set via `contains_key`. Zero-backcompat: field type changes in one
patch, all readers update in the same diff.

**Verification.** Test: 3 full-file reads on one path → response 2 has
`SUGGEST_OUTLINE`, response 3 has `suggested_call`.

### P3.4 — Pre-edit `path_rename` nudge via the existing hook

**Evidence.** `SessionState.use_prefix_edits: HashMap<String, u64>` already
tracks hot prefixes. `codesight-mcp hook-pretooluse` already exists. No
new binary, no new subcommand needed.

**Fix.** In the existing `hook-pretooluse` implementation, read the pending
Edit/Write's `file_path` and `new_string`. If `new_string` contains a `use `
line whose prefix matches a hot entry (`use_prefix_edits[prefix] >= 3`
across `>= 3` distinct files in this session), append an
`additionalContext` snippet suggesting `evaluate_impact
analysis=path_rename from="<hot_prefix>::..." to="..."`. Persist the
counter bump from the existing `PostToolUse` hook — already done.

Zero new files, zero new config surface; hot-prefix detection becomes a
predicate on state that already exists.

**Verification.** Manual: three hand use-prefix edits across three files;
fourth Edit sees a `path_rename` suggestion in `additionalContext`.

---

## Not doing (explicit non-scope)

- New `SessionSignals` event log / "signals bus" abstraction. Every symptom
  above has a 1-line fix against existing state. Adding a generic event log
  would be a new primitive the server has to maintain forever with nothing
  measurable gained.
- Renaming `agent_readiness_score` → `agent_readiness`. Semantic fix only;
  name stays.
- New `include_conventional` knob for NL search. Handled via BM25 retune +
  directory prior.
- Criterion bench gate for cold ingest. Direct timing is authoritative;
  1.55 s vs 5 s budget leaves enough headroom that CI gating costs more
  than it protects.

---

## Work packaging

| PR | Scope | Items | Est. effort |
|---|---|---|---|
| 1 | Trust | P0.1, P1.1 | 2–3 h |
| 2 | Correctness | P0.3, P1.2, P2.3 | ½ day |
| 3 | Signal | P2.1, P2.2, P3.2 | ½ day |
| 4 | Polish | P3.1, P3.3, P3.4 | 2–3 h |

Each PR ends with `manage_indexes action=session_end`; gate on
`quality_delta.verdict != "regressed"`.

## Session quality gate

Same as before:
- `cargo nextest run` green.
- `evaluate_quality analysis=quality_signal scope="<touched crate>"` shows
  no previously passing threshold newly failing.
- `session_stats` — no new anti-pattern entries and
  `read_efficiency.score >= 0.70`.

Net target after all four PRs: fresh self-dogfood with zero empty-in-flight
`STALE_DATA_WARNING`s, NL ranking dominated by `src/`, rename safety cover,
and cold ingest still <2 s.
