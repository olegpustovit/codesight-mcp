; language: tsx
; extensions: tsx
; grammar: tsx

; ── Classes ───────────────────────────────────────────────
(class_declaration name: (type_identifier) @name) @definition.class

; ── Class methods ─────────────────────────────────────────
(class_declaration
  body: (class_body
    (method_definition name: (property_identifier) @name) @definition.method))

; ── Functions ─────────────────────────────────────────────
(function_declaration name: (identifier) @name) @definition.function
(generator_function_declaration name: (identifier) @name) @definition.function

; ── Const arrow / function expressions → Function ─────────
(lexical_declaration
  (variable_declarator
    name: (identifier) @name
    value: (arrow_function)) @definition.function)

(lexical_declaration
  (variable_declarator
    name: (identifier) @name
    value: (function_expression)) @definition.function)

(lexical_declaration
  (variable_declarator
    name: (identifier) @name
    value: (generator_function)) @definition.function)

; ── Const values → Constant ───────────────────────────────
(lexical_declaration
  (variable_declarator
    name: (identifier) @name) @definition.constant)

; ── Interfaces & type aliases ─────────────────────────────
(interface_declaration name: (type_identifier) @name) @definition.type
(type_alias_declaration name: (type_identifier) @name) @definition.type
