; language: bash
; extensions: sh bash
; grammar: bash

; ── Functions ─────────────────────────────────────────────
(function_definition name: (word) @name) @definition.function

; ── Constants (ALL_CAPS variable assignments) ─────────────
(variable_assignment name: (variable_name) @name
  (#match? @name "^[A-Z][A-Z0-9_]+$")) @definition.constant
