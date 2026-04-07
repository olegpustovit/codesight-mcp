; language: fortran
; extensions: f f90 f95 f03 f08 for fpp
; grammar: fortran

; ── Doc comments (! style) ──────────────────────────────
; Standalone @doc: the engine associates preceding doc comments with the
; next symbol by byte proximity when no containing symbol is found.
; Works for top-level functions/subroutines. For methods inside modules,
; inline @doc captures in the method patterns below take precedence.
(comment) @doc

; ── Modules ─────────────────────────────────────────────
; Modules are extracted as class-kind symbols (matching class-kind module extraction).
(module
  (module_statement
    (name) @name)) @definition.class

; ── Methods with doc (functions/subroutines inside modules) ──
; Inline @doc captures prevent the standalone doc from being mis-assigned
; to the containing module. Listed BEFORE plain methods so they win dedup.
(module
  (internal_procedures
    (comment) @doc
    (function
      (function_statement
        name: (name) @name)) @definition.method))

(module
  (internal_procedures
    (comment) @doc
    (subroutine
      (subroutine_statement
        name: (name) @name)) @definition.method))

; ── Methods without doc (functions/subroutines inside modules) ──
(module
  (internal_procedures
    (function
      (function_statement
        name: (name) @name)) @definition.method))

(module
  (internal_procedures
    (subroutine
      (subroutine_statement
        name: (name) @name)) @definition.method))

; ── Functions (top-level) ───────────────────────────────
(function
  (function_statement
    name: (name) @name)) @definition.function

; ── Subroutines (top-level) ─────────────────────────────
(subroutine
  (subroutine_statement
    name: (name) @name)) @definition.function

; ── Derived types ───────────────────────────────────────
(derived_type_definition
  (derived_type_statement
    (type_name) @name)) @definition.type

; ── Parameter constants ─────────────────────────────────
; `integer, parameter :: MAX_SIZE = 100`
(variable_declaration
  (type_qualifier) @_attr
  (init_declarator
    left: (identifier) @name)
  (#eq? @_attr "parameter")) @definition.constant
