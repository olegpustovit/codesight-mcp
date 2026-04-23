# PLAN.md — codesight-mcp dogfooding remediation

Concrete implementation plan for issues identified in the 2026-04-22 dogfooding
session. Ordered by priority. Each item: scope, files, approach, verification.

Session evidence referenced below comes from `manage_indexes session_stats` and
`manage_indexes analysis_health` run against commit `18aab4e`.

---

## P0 — correctness / trust

### P0.1 — Silence `STALE_DATA_WARNING` when nothing is in flight

**Problem.** `session_stats.stale_warnings_fired_with_in_flight_paths` shows
~100 entries where `files_in_flight: []`. The warning fires on the first
`catalog_sources` of every session even when no reindex is live. Agents learn
to ignore it, so real staleness gets ignored too.

**Files.**
- `crates/codesight-engine/src/service/agent_v1/freshness.rs` (emitter)
- `crates/codesight-engine/src/service/mod.rs` (warning wiring)

**Approach.**
1. Locate the warning emitter (grep `STALE_DATA_WARNING` in
   `codesight-engine`).
2. Make emission conditional on `!files_in_flight.is_empty() ||
   indexing_started_ago_ms < GRACE_MS` (GRACE_MS = 500).
3. Collapse consecutive identical warnings inside one session: keep a
   `last_warning_fingerprint` on the session ledger and skip duplicates.
4. Add a `reason` discriminator so callers can tell `in_flight_reindex` from
   `snapshot_age_exceeded` from `pending_migration`.

**Verification.**
- Add `tests/stale_warning_suppression.rs`: ingest → call `catalog_sources`
  twice in a row → assert no `STALE_DATA_WARNING` on the second call.
- Re-run `session_stats` after 10 tool calls in a quiet repo; assert
  `stale_warnings_fired_with_in_flight_paths` is empty.

### P0.2 — Restore cold-ingest budget (<5s on this repo)

**Problem.** `ingest_sources` took 28.6s cold on 544 files; AGENTS.md §5 budget
is <5s. ~6× regression.

**Files.**
- `crates/codesight-engine/src/indexing/code_index.rs`
- `crates/codesight-engine/src/indexing/dep_graph.rs`
- `crates/codesight-extractors/src/extractors/engine/mod.rs`

**Approach.**
1. Add tracing spans around the four phases of `code_index::ingest`
   (walk / parse / extract / persist) with `tracing::info_span!`.
2. Run once with `RUST_LOG=codesight_engine::indexing=info` to identify the
   bottleneck phase.
3. Likely suspects given recent churn:
   - **Parse.** tree-sitter parser recreation per file — cache
     `tree_sitter::Parser` in a thread-local or per-worker pool.
   - **Persist.** per-symbol inserts vs prepared-statement batch — ensure one
     transaction, `INSERT ... VALUES (...),(...),(...)` up to SQLite's
     999-param cap.
   - **Walk.** `ignore` crate config — confirm `.gitignore` walker isn't
     re-stat'ing files.
4. Write a criterion bench in
   `crates/codesight-engine/benches/cold_ingest.rs` that ingests a 500-file
   fixture and fails if >5s.

**Verification.**
- Bench gate: `cargo bench -p codesight-engine --bench cold_ingest` < 5s.
- Re-run `ingest_sources path=.` on this repo, expect `timing_ms < 5000`.

### P0.3 — Fix `evaluate_impact analysis=path_rename` prefix boundary

**Problem.** Asked to rewrite `codesight_language_support::traits::Language →
codesight_language_support::Language` returned `changes:0, files_affected:[]`,
yet the session log shows I did the same rewrite by hand across ~60 sites.
The byte_scan matcher is missing real hits.

**Files.**
- `crates/codesight-analysis/src/impact/path_rename.rs`
- `tests/path_rename.rs` (new)

**Approach.**
1. Audit the boundary predicate. The matcher almost certainly requires one of
   `::`, `{`, `;`, whitespace immediately after `from`. Add checks for
   `)` (generic bound position), `,` (import group), `>` (turbofish closer),
   `\n` (line break inside a `use` group).
2. Handle grouped `use` imports: `use codesight_language_support::traits::{
   Language, Other };` — the `from` prefix ends mid-group; `handle_partial_
   group_imports` already has the enum; make it actually work.
3. Handle re-exports (`pub use`) — same matcher, same result expected.
4. Add a golden test file with 15 canonical shapes (plain `use`, grouped
   `use`, `pub use`, `as` alias, fully-qualified call site,
   `impl Trait for Type` bound, turbofish, etc.) and assert the rewrite
   count matches.

**Verification.**
- `cargo nextest run --test path_rename` — all 15 shapes matched.
- Dry-run against the same prefix that returned 0 on this session; expect
  non-zero `files_affected`.

---

## P1 — efficiency / ergonomics

### P1.1 — Session-sticky `catalog_sources`

**Problem.** 41 calls this session, ~5s cumulative, `tokens_saved=0`. Policy
forces start-of-session call but the payload never changes unless an ingest
fires.

**Files.**
- `crates/codesight-engine/src/service/mod.rs` (session state)
- `crates/codesight-engine/src/service/agent_v1/catalog_sources.rs` (handler)

**Approach.**
1. Add `session.catalog_cache: Option<(indexed_at_max, CatalogPayload)>`.
2. On `catalog_sources`, compare the cached `indexed_at_max` against a cheap
   `SELECT MAX(indexed_at) FROM sources` (indexed column). If equal, return
   cached payload with `_meta.cache_hit: true` and `timing_ms < 5`.
3. Invalidate the cache in `ingest_sources` and `manage_indexes
   action=invalidate/delete/rebuild`.

**Verification.**
- New test: call `catalog_sources` 10× in a row → first call touches SQLite,
  calls 2..10 return `cache_hit: true`.
- `session_stats.per_tool.catalog_sources.timing_ms_sum` drops an order of
  magnitude in the self-dogfood test.

### P1.2 — Fuzzy search normalizes case style before trigrams

**Problem.** `query="resolveImprts"` (typo + camelCase) with `fuzzy=true`
returned 0 hits against `resolve_imports_to_files`. Trigrams compare raw
strings; snake↔camel never reconciles.

**Files.**
- `crates/codesight-retrieval/src/fuzzy.rs` (likely location)
- `crates/codesight-retrieval/src/ngram.rs` or wherever trigram tokens are
  built.

**Approach.**
1. Introduce `normalize_identifier(&str) -> String`:
   lowercase, strip non-alphanumeric, no separators. `resolveImprts` →
   `resolveimprts`, `resolve_imports_to_files` → `resolveimportstofiles`.
2. Build trigrams over the normalized form on *both* query and indexed
   symbol names.
3. Keep the raw form stored for display/return; normalization is only for
   comparison.
4. Edit-distance cutoff applies on the normalized form too.

**Verification.**
- Add tests in `crates/codesight-retrieval/src/fuzzy.rs`:
  - `resolveImprts` matches `resolve_imports_to_files` within threshold.
  - `MyStruct` matches `my_struct` (kebab edge case).
  - `foo_bar` still matches `foo_bar` with score 1.0.

---

## P2 — signal quality

### P2.1 — Down-weight tests/benches/fixtures in `query_mode=natural_language`

**Problem.** NL query *"how does import resolution work across languages?"*
returned top-5 results all from `tests/find_importers.rs`. Test files have
dense repeated keywords; they swamp real implementation.

**Files.**
- `crates/codesight-retrieval/src/bm25.rs` or the NL ranker
- `crates/codesight-engine/src/service/agent_v1/catalog_search/mod.rs`

**Approach.**
1. Attach a `file_class` tag to indexed files during ingest:
   - `test` — path starts with `tests/`, or matches `*_test.rs`,
     `test_*.py`, `*.spec.ts`.
   - `bench` — `benches/`, `*.bench.*`.
   - `fixture` — `fixtures/`, `testdata/`, `__fixtures__/`.
   - `script` — `scripts/`.
   - `prod` — else.
2. In NL ranking, multiply the final score by a per-class weight:
   `prod=1.0, test=0.35, bench=0.35, fixture=0.2, script=0.5`.
3. Gate behind `include_conventional` (mirror the existing
   `evaluate_quality` knob) — defaults off, users can opt back in.
4. Echo the applied weights in `_meta.weighting_applied` for transparency.

**Verification.**
- Integration test: NL query on a fixture repo with identical keyword
  density in `src/` and `tests/` must rank `src/` first.
- Re-run the original query manually; top 5 should now be from
  `crates/codesight-*/src/`.

### P2.2 — Per-variant error counts in `inspect_graph`

**Problem.** `session_stats.per_tool.inspect_graph.error_count=3` on 19 calls
(~16%), no visibility into which `graph=` enum failed.

**Files.**
- `crates/codesight-engine/src/service/agent_v1/graph.rs` (dispatcher)
- `crates/codesight-engine/src/service/savings.rs` (telemetry sink)

**Approach.**
1. Add `per_tool[tool].error_count_by_variant: HashMap<String, u64>` where
   the variant key is the `graph=` enum value (`references`, `calls`,
   `imports`, `importers`, `dependencies`, `dependency_cycles`,
   `centrality`, `module_coupling`, `layer_violations`, `find_references`,
   `resolve_fqn`).
2. Wire the variant tag from dispatcher into the savings sink alongside the
   existing `tool` label.
3. Apply the same pattern to `inspect_structure view=`, `evaluate_quality
   analysis=`, `evaluate_impact analysis=` — all consolidated tools need
   variant telemetry.

**Verification.**
- `session_stats` response contains `error_count_by_variant` when any
  variant errored.
- Integration test: deliberately call `inspect_graph graph=calls
  symbol_id=does_not_exist` → assert `error_count_by_variant.calls == 1`.

### P2.3 — `quality_signal.recommendations.first_step` either works or is absent

**Problem.** Returned `first_step: { tool: "search_catalog", args: { query: "",
unit_kinds: ["symbol"] } }`. An empty `query` is a no-op.

**Files.**
- `crates/codesight-analysis/src/quality_signal/recommendations.rs`

**Approach.**
1. For each recommendation, require a concrete seed before emitting
   `first_step`. If no seed is available, emit just `suggestion` and drop
   `first_step`.
2. For the `symbol_uniqueness` recommendation (duplicate name disambiguation),
   seed with the most-duplicated name from the scope's offender list
   (e.g. `"extract"` or `"visit"`).
3. For the `change_locality` recommendation, seed `evaluate_quality
   analysis=refactor_targets` with the top offender's `file_path`.

**Verification.**
- `tests/quality_signal.rs` — for each recommendation returned, either
  `first_step` is `None` or `first_step.args` has no empty string / empty
  array fields.

---

## P3 — polish

### P3.1 — Unify `agent_readiness_score` encoding

**Problem.** Same response emits `agent_readiness_score: 6803` (int
thousandths) and `agent_dimensions.change_safety: 0.61` (float).

**Files.**
- `crates/codesight-analysis/src/quality_signal/mod.rs`
- `crates/codesight-analysis/src/agent_readiness.rs`

**Approach.** Pick float `[0.0, 1.0]` everywhere. Delete every `* 10_000`
site. Update `tests/quality_signal.rs` and any snapshot fixtures. Zero
backwards compatibility — rename the field to `agent_readiness` (no
`_score`) to force every caller through the type system.

**Verification.** `cargo nextest run` clean; `search_catalog
query="agent_readiness_score"` returns zero hits after the rename.

### P3.2 — Reconcile `analysis_health` docs with response shape

**Problem.** AGENTS.md §Working rules references
`unresolved_use_statements` as a red-flag metric; the actual
`manage_indexes action=analysis_health` response omits it.

**Files.**
- `crates/codesight-engine/src/service/agent_v1/index_admin.rs`
- `AGENTS.md` (if we decide the doc was aspirational)

**Approach.**
1. Decide: surface the metric or drop the doc reference.
2. If surface: compute from the import resolver — count import edges whose
   `resolution_status = Unresolved`. Add to the `graph` block of the
   response.
3. If drop: remove the bullet from AGENTS.md and open a separate ticket for
   the unresolved-imports feature.

Recommend **surface**, since it's the most useful of the three signals the
doc promised.

**Verification.** Test that calls `analysis_health` on a repo with a known
bad import → `graph.unresolved_use_statements > 0`.

### P3.3 — Nudge on repeated full-file `read_content`

**Problem.** 121 of 148 reads (82%) were full-file. `read_efficiency` score
0.59. 33 `TOKEN_HEAVY_READ` warnings fired and were silently ignored.

**Files.**
- `crates/codesight-engine/src/service/agent_v1/content.rs`

**Approach.**
1. Track per-session `full_file_reads_by_path: HashMap<String, u8>`.
2. On the 2nd full-file read of the same file, attach to the response:
   ```json
   {"warnings": [{"code": "SUGGEST_OUTLINE",
     "message": "read this file twice as full — consider inspect_structure view=file_outline first, then batch symbol refs"}]}
   ```
3. On the 3rd+, promote to a harder warning and include a prebuilt
   `suggested_call` that the agent can copy.

**Verification.** Test: call `read_content` with a full-file ref on the same
path 3× in one session → response 2 has `SUGGEST_OUTLINE`, response 3 has
`suggested_call`.

### P3.4 — Pre-edit `path_rename` guardrail hook

**Problem.** `session_stats` flagged `HAND_ROLLED_PATH_RENAME` only
post-hoc — after 60+ hand edits. The anti-pattern detector should fire
*before* the N+1 edit.

**Files.**
- New: `src/cli/hooks/pre_edit_path_rename.rs`
- `crates/codesight-engine/src/service/savings.rs` (shared rolling counter)

**Approach.**
1. In the edit-tracking sink, when ≥3 Edit/Write calls within the same
   session touched the same `use PREFIX::...` prefix across ≥3 distinct
   files, mark the prefix as "hot."
2. Expose a hook command (`codesight-mcp hook pre-edit`) that reads the
   pending edit's target file + new content, detects any hot prefix, and
   emits `additionalContext` pointing to
   `evaluate_impact analysis=path_rename from="..." to="..."`.
3. Ship an example `settings.json` snippet in `README.md` that wires the
   hook as a `PreToolUse` matcher for `Edit`/`Write`.

**Verification.** Manual session: do 3 hand use-prefix edits across 3 files,
then on the 4th edit the hook emits a `path_rename` suggestion in
`additionalContext`.

---

## Work packaging

Four PRs, each self-contained:

| PR | Scope | Items | Estimated effort |
|---|---|---|---|
| 1 | Trust & noise reduction | P0.1, P1.1 | half day |
| 2 | Correctness | P0.3, P1.2, P2.3 | one day |
| 3 | Performance | P0.2 | one day (investigation-heavy) |
| 4 | Signal & telemetry | P2.1, P2.2, P3.1, P3.2, P3.3, P3.4 | one day |

Each PR ends with `manage_indexes action=session_end` and verifies
`quality_delta.verdict != "regressed"`.

---

## Session quality gate for this plan

Before merging any PR:
- `cargo nextest run` full suite green.
- `evaluate_quality analysis=quality_signal scope="<touched crate>"`
  before/after — no threshold that was passing may newly fail.
- `manage_indexes action=session_stats` — no new anti-pattern entries and
  `read_efficiency.score >= 0.70`.

Net target after all four PRs: a fresh self-dogfood session ends with
`tokens_saved_total > 100M`, zero `STALE_DATA_WARNING` noise, cold ingest
`<5s`, and `verdict != "regressed"`.
