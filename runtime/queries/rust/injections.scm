; injections.scm — Rust embedded languages
;
; Recognized by `extractors::engine::scm::injections::run_injections`. Content spans are
; delegated to the language named by `#set! injection.language "..."` via the shared
; extractor registry.

; ── sqlx / generic `sql!` macros → SQL ────────────────────
((macro_invocation
   macro: (scoped_identifier
     name: (identifier) @_macro)
   (token_tree
     (string_literal (string_content) @injection.content)))
 (#any-of? @_macro "query" "query_as" "query_scalar" "query_file")
 (#set! injection.language "sql"))

((macro_invocation
   macro: (identifier) @_macro
   (token_tree
     (string_literal (string_content) @injection.content)))
 (#eq? @_macro "sql")
 (#set! injection.language "sql"))
