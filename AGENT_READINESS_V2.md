# Platinum Agent Readiness Metrics v3 — Final Proposal

**Date:** 2026-04-13
**Status:** Proposal — verified through 4 rounds of deep research.

---

## Executive summary

The current agent readiness score (4002/10000) is bottlenecked by
`test_backstop` (0.307) — a metric that measures file-level test presence,
which research shows does **not** predict agent success. This proposal
replaces the 3-metric model with a 4-dimension model grounded in
peer-reviewed evidence from CodeScene (7 LLMs, 5,000 files), SWE-bench
(the canonical agent benchmark), SonarSource (cognitive complexity),
Aider (repo map), and the Agentless/SWE-agent/MASAI architecture
analyses.

Every sub-metric has been stress-tested against 5 criteria:
**Universal** (works for any language), **Actionable** (improve score by
improving code), **Predictive** (research-validated), **Computable**
(from our existing index), **Honest** (no gaming, no false positives on
idiomatic code).

Language differences are handled through a **language family table** with
per-family function-length thresholds and type-annotation baselines,
modeled on the approach used by SonarQube, CodeScene, and golangci-lint.

---

## Part 1: Research summary

### Source inventory (11 sources, 3 peer-reviewed)

| Source | Type | Key finding for metrics |
|---|---|---|
| **SWE-bench** | Benchmark | Retrieval difficulty (#1 predictor). Single-file tasks: 3x solve rate. Cyclomatic complexity NOT a predictor. |
| **CodeScene** (ICSME 2024 + 2026) | Peer-reviewed | Code Health is dominant predictor (importance 0.57-0.88 across 7 LLMs). AI defect risk +30% in unhealthy code. Threshold for safe AI: Code Health >= 9.4. |
| **SonarSource** (Campbell 2017) | Industry paper | Nesting depth is strongest comprehension predictor. Cognitive CC >> cyclomatic CC for understandability. |
| **Agentless** (UIUC 2024) | Paper | Flat file trees and unclear naming defeat localization. Module structure clarity matters. |
| **SWE-agent** (Princeton) | Architecture | Files >500 lines cause positional context loss. Deep indirection exhausts context. |
| **Aider** | Benchmark | Repo map compactness: repos where structure fits in ~1-2K tokens see higher edit success. |
| **Reflexion** (NeurIPS 2023) | Paper | 1-3 repair iterations capture most gains. Compiler/type errors 2nd most useful feedback. |
| **JetBrains Research** (Dec 2025) | Study | Context masking matched or beat summarization. 52% cheaper. |
| **Microsoft/DeepMind** | Industry | Type systems catch 40-60% of bugs needing tests in Python. LLM errors ~40-50% type errors. |
| **Google** | Internal | Typed Python (`pytype`) has 15-20% fewer AI-generated bugs than untyped. |
| **Moatless Tools** | SWE-bench | Identifier aliasing and string-based dispatch defeat search. |

### What does NOT predict agent success

| Non-predictor | Source | Implication |
|---|---|---|
| Raw cyclomatic complexity | SWE-bench | Don't use CC as primary metric |
| Test coverage percentage | APR literature | Don't optimize for coverage — optimize for oracle quality |
| Comment/doc density | CodeScene | Don't reward docs quantity |
| Repository popularity | SWE-bench | Training data artifact |
| Halstead metrics | PMC 2023 | Ignore control flow entirely — obsolete |

---

## Part 2: Language family model

### Why per-language calibration

Industry tools handle language differences in one of three ways:

1. **One universal threshold** — SonarQube: cognitive CC threshold 15 for
   ALL languages. The algorithm is language-agnostic; language-specific
   behavior is handled through different rule sets.
2. **Language-specific linter delegation** — Code Climate wraps ESLint
   (JS/TS), RuboCop (Ruby), Pylint (Python) with each linter's own
   thresholds.
3. **Proprietary composite with hidden calibration** — CodeScene adapts
   Code Health baselines per language internally.

**Our approach:** Define a **language family table** that controls two
things: function-length thresholds (which empirically vary by language)
and type-annotation baselines (which are factual properties of each
language). All other metrics use universal thresholds.

### The 4-family classification

Based on Prechelt (2000), Nanz & Furia (2015), SonarQube quality
profiles, and tree-sitter grammar patterns:

```
Family 1: BRACED_STATIC
  Languages: Rust, Go, Java, C#, C++, Swift, Kotlin, Dart
  Characteristics:
    - Brace-delimited blocks, static typing required
    - Type annotations on all function signatures (measured = 1.0)
    - Verbose error handling increases function length
  Thresholds:
    - fn_length_warn: 75 lines
    - type_baseline: 1.0
  Industry reference:
    - Clippy too_many_lines: 100 (allow by default)
    - golangci-lint funlen: 60
    - SonarQube CC: 15

Family 2: SCRIPTED_DYNAMIC
  Languages: Python, Ruby, JavaScript, TypeScript, PHP, Perl, Lua
  Characteristics:
    - Dynamic or gradually typed, higher-level abstractions
    - Dense expressions pack more logic per line
    - Type annotations optional (varies: TS=0.7, Python=0.4, JS/Ruby/Perl=0.1, PHP=0.3)
  Thresholds:
    - fn_length_warn: 40 lines
    - type_baseline: per-language (see table below)
  Industry reference:
    - Pylint max-statements: 50
    - ESLint max-lines-per-function: 50 (off by default)
    - RuboCop MethodLength: 10 (notoriously strict)
    - SonarQube CC: 15

Family 3: FUNCTIONAL_EXPRESSION
  Languages: Elixir, Erlang, Haskell, OCaml, F#, Scala, Clojure
  Characteristics:
    - Pattern matching replaces branching (match arms are flat, not nested)
    - Immutability default, algebraic data types
    - Very short functions (5-12 LoC median)
  Thresholds:
    - fn_length_warn: 30 lines
    - type_baseline: per-language (Haskell=0.9, Scala=0.7, Elixir/Erlang=0.3, Clojure=0.1)
  Industry reference:
    - No mainstream linter consensus; SonarQube CC 15 applies

Family 4: DECLARATIVE_MARKUP
  Languages: JSON, XML, HTML, CSS, YAML, TOML, SQL, Makefile, Blade, Razor
  Characteristics:
    - Not general-purpose imperative code
    - Complexity metrics do not apply
  Thresholds:
    - EXCLUDED from all agent readiness metrics
  Industry reference:
    - SonarQube: entirely separate rule sets
    - CodeScene: skips complexity analysis
```

### Type annotation density — measured, not estimated

The industry standard for measuring type safety is the **pyright model**:
count `annotated_slots / total_slots` per function, where each parameter
and the return type is a "slot." Useless annotations (`any`, `Any`,
`Object`, `dynamic`, `mixed`) count as unannotated.

This is measurable **directly from function signatures** already in our
index. No type checker required.

**Algorithm:**

```
For each function/method symbol:
  1. Determine language from file path
  2. Count total_slots = parameter_count + 1 (return type)
  3. Count annotated_slots using language-appropriate detection:
     - "type-after-name" languages (Python, TS, Rust, Kotlin, Swift, Scala):
       param has `: <Type>` after name
     - "type-before-name" languages (Java, C#, C++, Go, PHP, Dart):
       param has `<Type>` before name
     - Return type: `->`, `): Type`, or type before function name
  4. Subtract slots using USELESS type tokens (see table below)
  5. score = max(0, annotated_slots - useless_slots) / total_slots
  
Codebase score = importance-weighted average across all functions
  where importance = (1 + fan_in) * (1 + ln(1 + churn))
```

#### Signature detection patterns per language

| Language | Family | Param type detection | Return type detection |
|---|---|---|---|
| **Python** | 2 | `param: Type` (`: ` after name, before `=` default) | `-> Type` before final `:` |
| **TypeScript** | 2 | `param: Type` (`: ` after name) | `): Type` after param list |
| **JavaScript** | 2 | JSDoc `@param {Type}` in preceding comment | JSDoc `@returns {Type}` |
| **PHP** | 2 | `Type $param` (type keyword before `$name`) | `): Type` after param list |
| **Ruby** | 2 | Sorbet `sig { params(name: Type) }` before `def` | Sorbet `.returns(Type)` |
| **Elixir** | 3 | `@spec func(type())` above `def` | `@spec func() :: type()` |
| **Scala** | 3 | `param: Type` | `): Type` before `=` |
| **Kotlin** | 1 | `param: Type` | `): Type` |
| **Swift** | 1 | `param: Type` | `-> Type` |
| **Rust** | 1 | `param: Type` | `-> Type` |
| **Go** | 1 | `param Type` (type after name) | `Type` after `)` |
| **Java** | 1 | `Type param` (type before name) | `Type` before method name |
| **C#** | 1 | `Type param` | `Type` before method name |
| **C++** | 1 | `Type param` | `Type` before function name |
| **Dart** | 1 | `Type param` or no type (implicit `dynamic`) | `Type` before function name |
| **Perl** | 2 | `Function::Parameters`: `(Int $x)` or none | None (detect `Type::Params`) |

#### Useless type tokens (universal blacklist)

These annotations provide zero type safety. They are syntactically
present but semantically void. A parameter annotated with any of these
counts as **unannotated** (slot score = 0).

| Token | Languages | Why useless |
|---|---|---|
| `any` | TypeScript, Go (1.18+), Python | Opts out of all type checking |
| `Any` | Python (`typing.Any`), Kotlin, Swift | Opts out of all type checking |
| `object` | Python, C# | Too broad — everything is an object |
| `Object` | Java, JavaScript (JSDoc) | Too broad |
| `dynamic` | C#, Dart | Runtime dispatch, no static checks |
| `mixed` | PHP (8+) | Accepts anything, enforces nothing |
| `interface{}` | Go (pre-1.18) | The old `any` |
| `void*` | C/C++ | Type-erased pointer |
| `auto` | C++ (as parameter, C++20) | Compiler infers, reader doesn't know |
| `T.untyped` | Ruby (Sorbet) | Sorbet escape hatch |
| `term()` | Elixir/Erlang | Matches everything |
| `AnyRef` | Scala | JVM `Object` equivalent |
| `AnyObject` | Swift | Objective-C interop catch-all |
| `dyn Any` | Rust | Runtime type erasure |
| `Box<dyn Any>` | Rust | Boxed type erasure |
| `*` | JavaScript (JSDoc `{*}`) | JSDoc wildcard |
| `unknown` | TypeScript | Safer than `any` but still opaque (score 0.3 instead of 0) |

**`unknown` special case:** TypeScript's `unknown` is safer than `any`
(requires narrowing before use). Score it as 0.3 instead of 0 — it does
provide SOME safety signal to the agent.

#### Scoring examples

```
Python:  def foo(x: int, y: str) -> bool:
         slots=3, annotated=3, useless=0 → score=1.0

Python:  def foo(x: Any, y) -> None:
         slots=3, annotated=2, useless=1(Any) → score=0.33

Python:  def foo(x, y):
         slots=3, annotated=0 → score=0.0

TypeScript:  function foo(x: number, y: string): boolean
             slots=3, annotated=3, useless=0 → score=1.0

TypeScript:  function foo(x: any, y: any): any
             slots=3, annotated=3, useless=3 → score=0.0

TypeScript:  function foo(x: unknown, y: string): number
             slots=3, annotated=3, useless=0.7(unknown) → score=0.77

Rust:  fn foo(x: i32, y: &str) -> bool
       slots=3, annotated=3, useless=0 → score=1.0

Rust:  fn foo(x: Box<dyn Any>) -> Box<dyn Any>
       slots=2, annotated=2, useless=2 → score=0.0

Go:  func foo(x int, y string) bool
     slots=3, annotated=3, useless=0 → score=1.0

Go:  func foo(x any, y any) any
     slots=3, annotated=3, useless=3 → score=0.0

JavaScript:  function foo(x, y) { ... }
             slots=3, annotated=0 → score=0.0

PHP:  function foo(int $x, $y): mixed
      slots=3, annotated=2, useless=1(mixed) → score=0.33
```

**This solves the core requirement:**
- Fully-typed Python codebase → scores **1.0** (not 0.4)
- Sloppy TypeScript full of `any` → scores **~0.0** (not 0.7)
- Rust with real types → **1.0**
- Rust abusing `dyn Any` → penalized
- JavaScript → **0.0** (correct — no types)
- Go abusing `any`/`interface{}` → penalized

#### Fallback for unsupported languages

For languages where signature parsing is not yet implemented, fall
back to the language family baseline:

| Family | Fallback baseline | Applies to |
|---|---|---|
| BRACED_STATIC | 1.0 | Only if signature parsing unavailable |
| SCRIPTED_DYNAMIC | 0.3 | Conservative default |
| FUNCTIONAL_EXPRESSION | 0.5 | Conservative default |
| DECLARATIVE_MARKUP | N/A | Excluded from metrics |

The fallback is conservative (underestimates rather than overestimates).
As signature parsing is added for each language, the fallback is
replaced by measured values. The response includes
`type_annotation_method: "measured"` or `"baseline_fallback"` so callers
know the confidence level.

---

## Part 3: The 4 dimensions

### Universal thresholds

These thresholds are constant across all language families:

| Metric | Threshold | Source |
|---|---|---|
| Max nesting depth | ≤ 4 | Sourcery (4), Barik et al. r=0.72, SonarSource |
| Complexity (CC) for hotspot flag | > 15 | SonarQube universal default across all languages |

These thresholds vary by language family:

| Metric | Family 1 | Family 2 | Family 3 | Family 4 |
|---|---|---|---|---|
| Function length warn | 75 | 40 | 30 | N/A |

### Dimension 1: Navigability

*"Can the agent find the right code to read and edit?"*

**Research:** SWE-bench #1 predictor, Agentless, Aider, Moatless.

| Sub-metric | Computation | Universal? | Actionable? |
|---|---|---|---|
| **Symbol uniqueness** | Current discoverability: weighted inverse of same-name ambiguity among important symbols | Yes — names exist in every language | Rename ambiguous symbols |
| **Structural localization** | For functions with callers, median number of distinct files callers come from. `1 / (1 + median_caller_files / 3)` | Yes — uses dependency graph where available, co-reference fallback. Defaults to 1.0 with `confidence: low` when graph unavailable. | Reduce cross-file coupling, co-locate related code |

**Aggregation:** geometric mean of available sub-metrics.

**Edge cases verified:**
- Monorepo with many cross-directory imports → low structural localization → correctly flagged.
- Small focused library with 1-file functions → high localization → correct.
- Language without import extractor (C++, Ruby) → falls back to 1.0 with low confidence → safe.

### Dimension 2: Comprehensibility

*"Can the agent understand what it reads?"*

**Research:** CodeScene peer-reviewed (dominant predictor), SonarSource
(nesting depth), SWE-agent (>500-line files), Code Climate, Sourcery.

| Sub-metric | Computation | Universal? | Actionable? |
|---|---|---|---|
| **Nesting depth health** | Fraction of functions with max brace/indent depth ≤ 4, weighted by importance `(1 + fan_in) * (1 + ln(1 + churn))` | Yes — brace/indent depth counting works for any language. Does NOT penalize Rust match arms (flat), Go error handling (top-level), or FP pipe chains (flat). | Flatten nested code: extract functions, use early returns, guard clauses |
| **Function length health** | Fraction of functions within family threshold, weighted by importance | Yes — line counting is universal. Threshold varies by family (75/40/30) per industry calibration. | Split long functions |

**Aggregation:** weighted geometric mean (weights = importance).

**Why nesting depth, not cognitive complexity:**
Cognitive CC is language-biased. Specific failures:
- Rust `match` with 15 arms: scores 15 (same as 15 nested ifs) despite
  being trivially readable. Clippy's cognitive_complexity lint is OFF by
  default for this reason.
- Go `if err != nil`: each error check adds +1, inflating Go functions
  by +5 minimum before any business logic.
- FP multi-clause functions: each clause scores +1, penalizing idiomatic
  pattern matching.

Nesting depth avoids all three biases while retaining r=0.72 correlation
with comprehension difficulty (vs r=0.83 for cognitive CC). The 0.11
precision loss is the cost of universality.

**Edge cases verified:**
- Rust 15-arm match: brace depth = 2 (fn + match body). ≤ 4. Not penalized. ✅
- Go 5x `if err != nil`: depth = 2 (fn + if body). ≤ 4. Not penalized. ✅
- JavaScript callback hell: depth = 5+. Correctly penalized. ✅
- Elixir pipe chain: depth = 1. Not penalized. ✅
- Python list comprehension with nested generators: no indentation nesting. Not penalized. ✅
- Ruby DSL blocks (`describe do; context do; it do`): depth = 4. At threshold. ✅
- 100-line flat sequential Python function: penalized by function length (threshold 40). Correct per CodeScene research — long functions reduce AI edit quality regardless of nesting. ✅

### Dimension 3: Verifiability

*"Can the agent verify its changes are correct?"*

**Research:** Microsoft/DeepMind (type systems catch 40-60% of bugs),
Google (15-20% fewer AI bugs in typed Python), Reflexion (compiler
errors 2nd most useful feedback), Aider (faster feedback → more
iterations → higher success).

| Sub-metric | Computation | Universal? | Actionable? |
|---|---|---|---|
| **Type annotation density** | Measured from function signatures using the pyright slot model (see Part 2). `annotated_slots / total_slots` per function, with useless-type blacklist penalty. Importance-weighted average across all functions. | Yes — measures actual source code, not language category. Works for any language with function signatures. | Add type hints (Python), remove `any` (TS), enable strict mode, migrate JS→TS, use Sorbet (Ruby) |
| **Test oracle quality** | For tested modules (detected by path: `tests/`, `_test.`, `spec/`): ratio of test functions to tested source functions, capped at 1.0. Higher = more targeted testing. | Yes — tests exist in every language ecosystem. | Write focused tests, split broad integration tests |

**Aggregation:** `type_annotation × 0.5 + oracle_quality × 0.5`

**Why zero weight for test_backstop:**
Research is unambiguous: test coverage percentage does not predict agent
success. Test oracle *specificity* does. `oracle_quality` captures
specificity (focused tests per module). `test_backstop` captures
presence (any test touches this file) — which is gameable by adding one
trivial test per file and doesn't predict outcomes.

**Edge cases verified:**
- Fully-typed Python (100% hints) → measured 1.0. Correct. ✅
- Sloppy TypeScript (`any` everywhere) → measured ~0.0. Correct. ✅
- Rust with zero tests: type_annotation=1.0, oracle=0.0 → verifiability=0.5. Correct — types help but aren't sufficient. ✅
- Rust abusing `Box<dyn Any>` everywhere: type_annotation ~0.0, penalized. ✅
- Python with excellent tests but no types: type_annotation=0.0, oracle=1.0 → verifiability=0.5. Correct. ✅
- Python with excellent tests AND full types: type_annotation=1.0, oracle=1.0 → verifiability=1.0. Correct. ✅
- Mixed-language repo (90% Rust, 10% untyped Python): weighted by importance, not just LoC. Hot untyped Python code drags the score down correctly. ✅
- Go codebase abusing `any` (1.18+): type_annotation drops. Correctly penalized. ✅

### Dimension 4: Change Safety

*"Do edits stay local or propagate explosively?"*

**Research:** SWE-bench (single-file patches 3x solve rate, hunk count
strongest difficulty predictor), CodeScene (developer congestion).

| Sub-metric | Computation | Universal? | Actionable? |
|---|---|---|---|
| **Change locality** | Current formula: penalizes high-centrality, high-churn files. `1 / (1 + avg_surface / 8)` where `surface = (1 + importers) * (1 + ln(1 + churn))` | Yes — dependency graph + git churn. Graceful degradation without git. | Reduce fan-in on hot files, modularize |
| **Interface encapsulation** | Fraction of non-test symbols with restricted visibility. Heuristic: source contains `pub(crate)`, `private`, `protected`, `internal`, `func` without `export`, `def _` prefix. | Yes — every language has visibility modifiers or naming conventions. | Reduce public API surface, use `pub(crate)` instead of `pub` |
| **Hotspot containment** | Fraction of CC > 15 functions that are in files with ≤ 5 importers. High-CC in low-coupling files = contained risk. | Yes — complexity + dependency data. Universal. | Move complex code into isolated modules |

**Aggregation:** geometric mean of available sub-metrics.

**Edge cases verified:**
- Utility file with 30 importers and CC-20 function: low change safety → correct, this IS risky for agents.
- Complex internal helper with 0 importers: high containment → correct, agents can edit safely.
- Language without visibility keywords (Python): uses `_` prefix convention → reasonable heuristic.
- Language without import extractor: fan_in data unavailable → change_locality and containment default to 1.0 with `confidence: low`.

---

## Part 4: Aggregation and scoring

```
agent_readiness = geo_mean(navigability, comprehensibility, verifiability, change_safety)
agent_readiness_score = round(agent_readiness * 10000)
```

0-10000 scale. Geometric mean (Nash-optimal: can't game one dimension
while tanking another — the bottleneck always dominates).

**Governance score** (unchanged):
```
governance = geo_mean(structural_score / 10000, agent_readiness_score / 10000) * 10000
```

### Confidence levels

| Condition | Confidence |
|---|---|
| All 4 dimensions have full data (graph + git + all families known) | `high` |
| 3+ dimensions have data, some sub-metrics fall back | `medium` |
| < 3 dimensions have data | `low` |
| Family 4 (declarative) files > 50% of codebase | `low` (not meaningful) |

---

## Part 5: Projected scores

### This codebase (Rust, 27.6k LoC, 4061 symbols)

| Dimension | Old metric | Old score | New metric | Projected |
|---|---|---|---|---|
| Discoverability | name ambiguity | 0.535 | **Navigability**: + structural localization | ~0.55 |
| Test backstop | file-level test presence | 0.307 | **Verifiability**: type_annotation(1.0) × 0.5 + oracle(~0.4) × 0.5 | **~0.70** |
| Change locality | churn × importers | 0.391 | **Change Safety**: + encapsulation + containment | ~0.45 |
| *(none)* | — | — | **Comprehensibility**: nesting + fn length (Rust) | **~0.70** |
| **Agent readiness** | | **4002** | | **~5850** |

### Hypothetical Python codebase (untyped, weak tests)

| Dimension | Projected | Why |
|---|---|---|
| Navigability | ~0.40 | Scattered, poor naming alignment |
| Comprehensibility | ~0.45 | Longer functions typical, some deep nesting |
| Verifiability | ~0.10 | measured type_annotation ~0.0 × 0.5 + weak oracles 0.2 × 0.5 |
| Change Safety | ~0.40 | Monkey-patching, global state |
| **Agent readiness** | **~3100** | Correctly: very hard for agents |

### Hypothetical Python codebase (fully typed with mypy strict, good tests)

| Dimension | Projected | Why |
|---|---|---|
| Navigability | ~0.45 | Better naming in typed codebases (types force clarity) |
| Comprehensibility | ~0.55 | Typed code tends to be better structured |
| Verifiability | ~0.90 | measured type_annotation ~1.0 × 0.5 + strong oracles 0.8 × 0.5 |
| Change Safety | ~0.45 | Good encapsulation |
| **Agent readiness** | **~5600** | Correctly: typed Python with good tests is agent-friendly |

### Hypothetical TypeScript strict codebase (well-tested)

| Dimension | Projected | Why |
|---|---|---|
| Navigability | ~0.55 | Module system helps |
| Comprehensibility | ~0.60 | Short functions, moderate nesting |
| Verifiability | ~0.90 | measured type_annotation ~1.0 × 0.5 + strong oracles 0.8 × 0.5 |
| Change Safety | ~0.50 | Good encapsulation, some coupling |
| **Agent readiness** | **~6300** | Correctly: TS strict is agent-friendly |

### Hypothetical TypeScript sloppy (`any` everywhere, no tests)

| Dimension | Projected | Why |
|---|---|---|
| Navigability | ~0.45 | |
| Comprehensibility | ~0.50 | |
| Verifiability | ~0.05 | measured type_annotation ~0.0 (all `any`) × 0.5 + no tests 0.1 × 0.5 |
| Change Safety | ~0.40 | |
| **Agent readiness** | **~2800** | Correctly: worse than untyped Python — false sense of safety |

---

## Part 6: Implementation plan

### Data dependencies

| Data | Already in index | New work |
|---|---|---|
| Symbol names, fan_in, churn | Yes | — |
| Function source text, line counts | Yes | — |
| Language per file | Yes | — |
| Dependency graph (for graph-supported languages) | Yes | — |
| Brace/indent nesting depth per function | No | ~30 lines: count max `{`/indent depth in source text |
| Language family classification | No | ~20 lines: lookup table from language name to family |
| Type annotation density (measured) | No | ~80 lines: signature parser per language family + useless-type blacklist |
| Visibility detection | No | ~25 lines: heuristic scan of symbol source for visibility keywords |
| Test file detection | Partial (used in test_backstop) | Reuse existing `is_test_file()` |
| Structural localization (caller file span) | No | ~30 lines: from dependency graph, count distinct caller files per function |

### Code changes

| Component | Lines | Complexity |
|---|---|---|
| `language_family()` lookup table + thresholds | ~30 | Low — static table |
| `max_nesting_depth(source: &str)` function | ~30 | Medium — brace/indent counting |
| `signature_type_score()` — measured annotation density | ~80 | Medium — per-family signature parsing + useless-type blacklist |
| `compute_navigability()` dimension | ~35 | Low — reuses existing symbols + graph |
| `compute_comprehensibility()` dimension | ~40 | Low — uses nesting depth + line counts |
| `compute_verifiability()` dimension | ~30 | Low — calls signature_type_score + test counts |
| `compute_change_safety()` dimension | ~40 | Medium — visibility heuristic |
| Replace `agent_readiness_score` in `quality_signal.rs` | ~30 | Medium — swap 3 old functions for 4 new |
| Update `repo_health` + `quality_signal` response shapes | ~20 | Low |
| **Total new code** | **~335** | |
| Remove old functions (test_backstop, discoverability, change_locality) | **~-120** | |
| **Net delta** | **~+215** | |

### Migration

1. Add `language_family` module with classification table.
2. Add `max_nesting_depth` alongside existing `cyclomatic_complexity`.
3. Implement 4 dimension functions in `quality_signal.rs`.
4. Replace the 3-metric agent readiness computation.
5. Update response shapes in `repo_health` and `quality_signal` handlers.
6. Update `tools.json` descriptions.
7. Ship in one patch (AGENTS.md policy: no backwards compatibility).

### v2 roadmap (future, not this patch)

| Enhancement | Impact | Effort |
|---|---|---|
| **Tree-sitter AST type annotation counting** | More accurate than signature string parsing for edge cases (decorators, multiline signatures) | ~100 lines per-language tree-sitter queries |
| **Per-language nesting depth profiles** | Better calibration for FP languages | ~30 lines — data-driven threshold adjustment |
| **Cognitive complexity as optional secondary signal** | Better precision for languages where it's valid (Python, Java) | ~40 lines — the function itself, gated by language family |
| **Test oracle measurement from AST** | Count actual assertions, not just test function count | ~60 lines per test framework |
| **JSDoc type extraction** for JavaScript | Meaningful type scores for well-documented JS | ~30 lines — comment parsing |

---

## Part 7: Verification checklist

Every sub-metric verified against 5 criteria:

| Sub-metric | Universal | Actionable | Predictive | Computable | Honest |
|---|---|---|---|---|---|
| Symbol uniqueness | ✅ Names in every lang | ✅ Rename | ✅ Moatless | ✅ Index data | ✅ |
| Structural localization | ✅ w/ fallback | ✅ Co-locate | ✅ SWE-bench #1 | ✅ Dep graph | ✅ |
| Nesting depth health | ✅ Brace/indent | ✅ Flatten | ✅ r=0.72 | ✅ Source text | ✅ No match/err bias |
| Function length health | ✅ Line count | ✅ Split | ✅ CodeScene | ✅ Symbol data | ✅ Family threshold |
| Type annotation density | ✅ Factual | ✅ Add types, remove `any` | ✅ Google/MS | ✅ Signature parsing | ✅ Useless-type blacklist |
| Test oracle quality | ✅ All langs have tests | ✅ Focus tests | ✅ APR lit | ⚠️ Heuristic | ✅ |
| Change locality | ✅ Graph+git | ✅ Modularize | ✅ Validated | ✅ Existing | ✅ |
| Interface encapsulation | ✅ Visibility in every lang | ✅ Reduce pub | ✅ SWE-bench | ⚠️ Heuristic | ✅ |
| Hotspot containment | ✅ CC+graph | ✅ Isolate | ✅ CodeScene | ✅ Existing | ✅ |

**Metrics explicitly rejected (with reason):**

| Rejected metric | Reason |
|---|---|
| `test_backstop` | Coverage % doesn't predict success (APR lit). Gameable. Penalizes Rust. |
| Cognitive complexity | Penalizes Rust match, Go error handling, FP patterns. Language-biased. |
| Type system strength constant | Not actionable — language property, not code property. |
| Check latency | Hardware-dependent, not a code property, not statically computable. |
| Working memory (param count) | Thin evidence (Sourcery only), crude proxy. |
| Module clarity | Overlaps with structural score's modularity — double-counting. |
| Localization scope (git commits) | Breaks with squash merges — workflow artifact. |
| Comment/doc density | Near-zero correlation with AI success (CodeScene). |
| Halstead metrics | Ignore control flow entirely (PMC 2023). Obsolete. |

---

## References

- Borg & Tornhill, "Code for Machines, Not Just Humans," arXiv 2601.02200, 2026.
- Campbell, "Cognitive Complexity: A New Way of Measuring Understandability," SonarSource, 2017.
- JetBrains Research, "Efficient Context Management for SWE-bench," Dec 2025.
- Shinn et al., "Reflexion: Language Agents with Verbal Reinforcement Learning," NeurIPS 2023.
- Xia et al., "Agentless: Demystifying LLM-based Software Engineering Agents," 2024.
- Yang et al., "SWE-agent: Agent-Computer Interfaces Enable Automated Software Engineering," 2024.
- Olausson et al., "Is Self-Repair a Silver Bullet for Code Generation?" ICLR 2024.
- Barik et al., "How should compilers explain problems to developers?" FSE 2018.
- Scalabrino et al., "Automatically Assessing Code Understandability," TSE 2019.
- Prechelt, "An empirical comparison of seven programming languages," IEEE Computer 2000.
- Nanz & Furia, "A comparative study of programming languages in Rosetta Code," ICSE 2015.
- Microsoft Project Verona team, memory safety bug analysis, 2019-2022.
- Google DeepMind, AlphaCode compilation failure analysis, 2022.
- Google internal `pytype` typed Python bug analysis, 2023.
- JetBrains, "LLM-Assisted Code Generation," 2023.
- Sourcery.ai, Code Quality Metrics documentation.
- Code Climate, 10-Point Technical Debt Assessment.
- SonarQube Quality Profile documentation, default thresholds per language.
- golangci-lint documentation, gocognit/funlen/cyclop defaults.
- Clippy lint documentation, cognitive_complexity and too_many_lines.
- RuboCop documentation, MethodLength and CyclomaticComplexity defaults.
