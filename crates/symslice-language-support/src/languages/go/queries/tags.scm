; language: go
; extensions: go
; grammar: go

; ── Functions ─────────────────────────────────────────────
(function_declaration name: (identifier) @name) @definition.function

; ── Methods (with receiver type as scope) ─────────────────
(method_declaration
  receiver: (parameter_list
    (parameter_declaration type: (type_identifier) @scope))
  name: (field_identifier) @name) @definition.method

(method_declaration
  receiver: (parameter_list
    (parameter_declaration type: (pointer_type (type_identifier) @scope)))
  name: (field_identifier) @name) @definition.method

; ── Struct / Interface types ──────────────────────────────
(type_declaration (type_spec
  name: (type_identifier) @name
  type: (struct_type))) @definition.class

(type_declaration (type_spec
  name: (type_identifier) @name
  type: (interface_type))) @definition.class

; ── Other type declarations ───────────────────────────────
(type_declaration (type_spec
  name: (type_identifier) @name)) @definition.type

; ── Constants ─────────────────────────────────────────────
(const_declaration (const_spec
  name: (identifier) @name)) @definition.constant
