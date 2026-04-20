; language: sql
; extensions: sql
; grammar: sql
; override_extractor: sql

; ── Tables / views / indexes / schemas ───────────────────
(create_table
  (object_reference) @name) @definition.type

(create_view
  (object_reference) @name) @definition.type

(create_index
  column: (identifier) @name) @definition.type

(create_index
  column: (literal) @name) @definition.type

(create_schema
  (keyword_schema)
  (identifier) @name) @definition.type

; ── Functions / CTEs ─────────────────────────────────────
(create_function
  (object_reference) @name) @definition.function

(cte
  (identifier) @name) @definition.function
