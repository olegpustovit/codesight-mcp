; language: lua
; extensions: lua
; grammar: lua

; ── Doc comments (--- style) ─────────────────────────────
; Standalone @doc: the engine associates preceding doc comments with the
; next symbol by byte proximity when no containing symbol is found.
(comment) @doc

; ── Colon-methods (Table:method) — use raw name as qualified_name ──
(function_declaration
  name: (method_index_expression
    method: (identifier) @name) @qualified_name) @definition.method

; ── Dot-methods (Table.func) — reference treats these as methods ──
(function_declaration
  name: (dot_index_expression
    field: (identifier) @name) @qualified_name) @definition.method

; ── Simple functions ──────────────────────────────────────
(function_declaration name: (identifier) @name) @definition.function

; ── Variable-assigned functions ───────────────────────────
(variable_declaration
  (assignment_statement
    (variable_list name: (identifier) @name)
    (expression_list value: (function_definition)))) @definition.function
