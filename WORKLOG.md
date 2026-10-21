# Worklog

## 2026-10-21 — empty impact results remain incomplete

Blast-radius confidence now describes collected evidence before applying the
requested file limit. Empty name searches and a zero limit cannot establish
safety: successful results carry `status: incomplete` and `complete: false`.
Name-based candidates remain partial, text-only evidence remains unreliable,
and unresolved structured references still return `NOT_FOUND`.

Four focused controls passed, one test each: empty/zero-limit results,
nonempty and weak-only evidence, structured-symbol impact, and missing
structured references. Workspace/all-target clippy and formatting passed.
The new empty-query dispatch timings were 0.20/0.23 ms; these are internal
dispatch measurements, not end-to-end SLA measurements.

Slow-test list: the four fixture tests took 2.70/3.44/4.08/1.96 s, each above
one second. Plan: reduce repeated cold fixture initialization while retaining
isolated state and every public-response assertion; do not ignore cases or
weaken checks to reduce setup time.

## 2026-10-21 — diagnostic development fixes

The five development findings from the SANER preparation were reproduced as
three targeted public-adapter regressions. Regex preserves matching lines and
honors its per-file cap; importer results separate resolved imports from heuristic
name-usage candidates and share a caller cap; symbol references and impact use
grammar identifier tokens rather than comments/string contents as primary evidence.
Name-binding resolution remains incomplete and is explicitly labeled partial.
Original unfavorable outputs remain in the private research workspace.

Targeted public regressions: 3 passed, 0 failed. Measured request times on the
small diagnostic fixture: regex 0.42/1.47 ms, importers 0.31 ms, references
1.18 ms, impact 0.56 ms. These timings establish fixture behavior only.
`cargo-nextest` is unavailable; `cargo test --test integration diagnostic_ -- --nocapture`
was used. Initial compile also exposed and removed an inherited stray `args`
reference in read-neighbor enrichment.

Slow-test list: the three diagnostic cases took 5.00 s together, mostly repeated
fixture ingestion. Keep public-adapter coverage; share the indexed fixture if
this setup cost grows. No tests were ignored or deleted.

Feature backlog: cache per-file identifier-token positions by indexed revision
before scaling reference scans; preserve explicit incomplete/binding labels.

An independent frozen-binary refresh test then exposed a stale safe read after
CLI preindex followed by a new server. Original results remain preserved.
Development correction attaches watchers to reopened stores, verifies public
symbol reads, rejects drifted offsets, and immediately warns/demotes overlapping
in-flight requests. A new CLI-preindex/Arc-service regression covers the observed
case; this is development verification, not an unseen independent evaluation.
The hooks CLI tests now invoke Cargo-provided CARGO_BIN_EXE directly, avoiding
recursive rebuilds and shared artifact replacement.

Generic stdio development probe preserved a separate liveness failure on rmcp1.5:
5/100 fresh-server trials stalled after separately flushed initialized and
tools/list messages;20/20 single-write trials passed. File-backed trace excluded
stderr backpressure and showed request parsing, response construction and a
ToSink event before missing stdout. This does not identify the exact internal
cause. Candidate correction upgrades to maintained rmcp3.6 stdio_pipes, whose
Linux FIFO path uses nonblocking Tokio pipes and retains upstream portable
fallback. A process-level25-start regression sends both frames without sleeps;
a separate100-start comparison is required before claiming the symptom fixed.

Candidate verification: matched normal-log/file-stderr probes completed90/100 on
rmcp1.5 and100/100 on rmcp3.6; all original failures remain preserved. The final
process regression passed25 fresh starts(1.91s); public contract/refresh16/16;
CLI/docs20/20; final workspace/all-targets clippy passed with warnings denied.
The earlier full workspace run passed1343 cases and failed only the now-corrected
docs classifier, whose focused group passed. No broad final full-suite pass is
claimed after the SDK upgrade; targeted transport/contract gates cover it.
Slow-test list: process regression1.91s; CLI-preindex refresh about3s; full gate
root integration294.07s. Keep the real process/I/O coverage; do not hide it with
ignored tests or client sleeps.

The owner-project study then exposed a public warning naming internal
candidate_text_hits even though references exposes weak_hits. The shared
warning now describes text-only candidates without naming any payload field,
and accurately notes limits/options can omit them. The existing public
reference regression checks that weak mentions remain accessible and the
internal field name is absent from warning text. Frozen316daf and all study
outputs remain preserved; the wording correction gets a separate executable.

Owner-project dogfood exposed unsupported-file false negatives, failed read items
reported as successes, and missing repository-scope disclosure. Graph paths now
require index membership; only outgoing imports require an extractor. Incoming
resolved edges may target any indexed file. Mixed reads classify per-item error
payloads before confidence/batch aggregation. Repo outline and health report
indexed-files-only coverage with INDEX_SCOPE_ONLY, registered language list,
unknown omission count, and partial or weaker confidence. Static test-gap reasons now say
no indexed test dependency; the existing count is explicitly a proxy, with no
executed-coverage claim. Original316 study outputs remain immutable.

Reference syntax parsing now skips source without the exact identifier substring;
weak case-insensitive text matching is unchanged. A direct debug-library diagnostic
on326 own Rust files produced identical identifier positions for three names,
reducing parses326 to35/2/10. This isolates parser work, not end-to-end SLA or a
cause of observed Rays latency. Shared-host elapsed numbers are retained in the
product-reference-prefilter-bench log.

Complexity cleanup removes duplicate cross-repository snapshots/store opens,
consolidates read-result classification and search truncation, and reuses source
resolution. Existing touched modules meet their baseline lexical metric; static
language-ID table growth is reported separately from control flow. New modules
and net production lines are measured in the fix report.

OCaml now uses maintained tree-sitter-ocaml0.26.0 with distinct implementation
and interface loaders for .ml/.mli. Generic SCM extracts structural functions,
modules, types and function signatures; OCaml identifier node classes exclude
comments/strings. No import resolution, complete bindings, PPX support or .rays
alias is claimed. Two development language regressions passed, including exact
source spans and nested interface/implementation names.

Dart signature queries now also capture the immediate sibling function_body.
The SCM collector joins same-kind captures into one definition span; mixed-kind
workspace packs retain last-definition behavior. The62-case SCM suite passed,
including block/arrow bodies and abstract declarations, and an invariant checks
that built-in patterns do not combine different definition kinds. A subsequent
full merge gate covers the final same-kind guard and public symbol reads.

Index coverage only demotes safe confidence; existing heuristic/unreliable labels
remain. Dead-code remains heuristic through the public health wrapper. Physical
production Rust prefix growth is a transparent exception for the owner-authorized
scope safeguards and actual OCaml support; no literal shrink-only pass is claimed.
No behavior is deleted or lines compressed to manufacture that result.

Crash-recovery validation found a separate watcher lifetime defect: dropping a
service detached its watch threads, allowing notify threads to accumulate across
serial tests. Watch handles now send stop and join the worker; the worker stops
and joins its debouncer. A 50-cycle lifecycle regression passed (6.10 s; real
notify shutdown exceeds the one-second test target). This does not identify the
cause of the earlier host crash. Freshness warnings now preserve heuristic and
unreliable labels rather than replacing them with partial.

The initial capped recovery gate was interrupted while progressing after PID-cap
denials; a completed corrected gate then exposed a new test's assumption that a
successful read always includes an empty errors array. The final span regression
uses exact outline-returned IDs and requires complete Dart/OCaml bodies. Final
workspace validation passed 1,355 tests with none failed/ignored under an aggregate
3 GiB memory/no-swap/128-task limit, one compiler worker and serial libtest.
Peak memory was 2,228,174,848 bytes, peak tasks 35, and memory/PID denial and OOM
counters were zero. All earlier incomplete and unfavorable logs remain preserved.
Formatting/whitespace checks and workspace/all-targets clippy with warnings denied
passed. The normal-profile binary build passed under the same caps, peak
2,501,730,304 bytes and 12 tasks, with zero memory/PID denial or OOM events.
The researcher keeps a copied frozen recovery executable; no commit or push was
performed by this recovery session.

Import resolution now combines extraction and resolution FactQuality. A generated
Go fixture demonstrates a standard-library name colliding with a local basename:
the guess remains a separate heuristic candidate, never a structural import or a
strict-analysis edge. Ambiguous basenames no longer pick an arbitrary HashSet
entry. Both import views share caps with candidate lists, retain totals and state
incomplete coverage. The resolver semantic stamp forces re-extraction on refresh;
deleted-path inventory is retained separately so the refresh removes old ghosts.

JSON key extraction now captures a whole key once and decodes its name with
serde_json. Escaped keys previously generated repeated fragment captures and
duplicated complete pair payloads. The generated before-test returned 15 symbols
for seven root pairs; the repaired test yields one per root pair with decoded
names/IDs and exact spans. Ordinary, quote/backslash, Unicode, empty, long escaped
and custom-scope controls pass, as does public outline-to-ID-to-read use. An exact
141,118-byte diagnostic input now indexes 459 symbols whose decoded names, IDs and
spans match an independent JSON check. This is development verification, not a
claim about every possible input or independent benchmark performance.

Focused import-quality and opt-in logging checks passed. Optional
`CODESIGHT_INGEST_DIAGNOSTICS=1` emits phase/file counts, grammar identity and Linux
RSS to tracing without source text. It is off by default; no data exclusion or
arbitrary size cap was introduced. Real-process logging and public-read tests take
several seconds and remain tracked as slow tests.

The combined workspace run completed 1,362 tests successfully and preserved three
failures: two assertions that still classified fallback DBT/Vue evidence as
structural, and a timing test that applied one-dispatch's 200 ms allowance to a
10,000-call aggregate. The exact DBT/Vue candidates now remain asserted together
with stored heuristic quality and strict-graph exclusion. The timing test keeps
200 ms/50 ms per-invocation guards and reports the 10,000-call observation without
pretending the aggregate is one dispatch. No runtime speedup is claimed. All three
corrected targets passed; the preceding full-run failure remains preserved, with
no new full-pass claim after these assertion-only edits. Workspace/all-target
clippy with warnings denied, formatting and whitespace checks passed.


## Import-evidence diagnostic correction

The published CI self-check failed after import candidates were correctly marked
heuristic. Its old diagnostic mislabeled all-language stored heuristic edges as
unresolved Rust uses. An isolated self-index found four Python candidates and
zero intersections with the strict graph; unresolved import coverage was not
measured. The original CI failure remains evidence of this diagnostic defect.

The diagnostic now reports `heuristic_import_edges`, actual
`heuristic_edges_in_strict_graph`, and `unresolved_import_coverage: not_measured`.
The redesigned `import_evidence_integrity` gate requires nonzero roots and zero
heuristic edges in the strict graph, failing closed on absent fields. This checks
evidence separation and does not replace the old intended Rust-completeness
check. No complete import-resolution claim follows from its result. Mixed-language
fixtures include two missing Rust specifiers alongside one Python heuristic candidate;
gate controls cover candidate acceptance, contamination, zero roots and missing
fields. Focused validation passed: two analysis-health tests, the gate control
test, and the stdio pipeline diagnostic. Workspace/all-target clippy with warnings
denied and formatting checks passed. This is targeted validation, not a new full
workspace or macOS CI pass.
