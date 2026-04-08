# Query Packs

`runtime/queries/<pack_id>/` is the canonical location for built-in symbol query packs.

Each pack directory uses this layout:

- `tags.scm` required. Defines symbol captures for the language.
- `injections.scm` optional. Reserved for host/container injection rules.
- `pack.toml` optional. Overrides or supplements header metadata.

`tags.scm` starts with semicolon-prefixed header metadata:

```scm
; language: rust
; extensions: rs
; grammar: rust
```

Supported header keys:

- `language`: logical language name. Defaults to the pack directory name.
- `extensions`: whitespace-separated file extensions without dots.
- `grammar`: compiled tree-sitter grammar name from `GrammarCatalog`.
- `override` or `override_extractor`: named residual extractor route when the pack should not compile directly into `ScmExtractor`.

`pack.toml` supports the same fields with TOML syntax:

```toml
language = "rust"
extensions = ["rs"]
grammar = "rust"
override_extractor = "yaml"
```

Metadata precedence:

- `tags.scm` header establishes defaults.
- `pack.toml` overrides matching fields when present.

Validation rules:

- every pack must define at least one `extensions` entry
- every pack must define either `grammar` or `override_extractor`
- unknown `pack.toml` keys are rejected

## Capture protocol

The baseline vocabulary follows Tree-sitter tags conventions:

- `@definition.<kind>` marks a symbol definition
- `@name` captures the display name for that symbol
- `@doc` captures inline or standalone documentation text

The engine also supports a small set of extensions needed by current `SymbolKind` and scope rules:

- `@qualified_name`: use the captured text as the emitted symbol name
- `@scope`: force the containing scope label for the symbol
- `#set! name_prefix "..."`: prepend a literal prefix to `@name`
- `#set! fixed_name "..."`: emit a literal name instead of reading `@name`
- `#set! scope_only`: create scope context without emitting a symbol
- `#set! exclude_inside "function"`: skip matches nested inside a specific definition family

Current `@definition.<kind>` values recognized by the engine:

- `function`
- `method`
- `class`
- `type`
- `constant`
- `module`
- `namespace`

`module` and `namespace` currently map to `SymbolKind::Type` in stored output, with scope behavior handled by the query engine.

## Adding a built-in pack

1. Create `runtime/queries/<pack_id>/tags.scm`.
2. Add header metadata, or supply it through `pack.toml`.
3. Use a `grammar` already present in `src/extractors/engine/catalogs/grammar.rs`.
4. Run `cargo nextest run --test query_pack_catalog`.

With an existing compiled grammar, that is enough for registry discovery. No `QUERY_FILES` or `registry_builder` edit is needed.
