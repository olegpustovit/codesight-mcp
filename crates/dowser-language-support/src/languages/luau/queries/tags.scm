; language: luau
; extensions: luau
; grammar: luau

; ── Doc comments ─────────────────────────────────────────
(comment) @doc

; ── Colon-methods (Table:method) ────────────────────────
(function_declaration
  name: (method_index_expression
    method: (identifier) @name) @qualified_name) @definition.method

; ── Dot-methods (Table.method) ──────────────────────────
(function_declaration
  name: (dot_index_expression
    field: (identifier) @name) @qualified_name) @definition.method

; ── Simple functions ────────────────────────────────────
(function_declaration name: (identifier) @name) @definition.function

; ── Type aliases / exported types ───────────────────────
(type_definition name: (identifier) @name) @definition.type
