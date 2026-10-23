; language: javascript
; extensions: js mjs cjs jsx
; grammar: javascript

; ── Classes ───────────────────────��───────────────────────
(class_declaration name: (identifier) @name) @definition.class

; ── Class methods ─────────────────────────────────���───────
(class_declaration
  body: (class_body
    (method_definition name: (property_identifier) @name) @definition.method))

; ── Functions ─────────────────────���───────────────────────
(function_declaration name: (identifier) @name) @definition.function
(generator_function_declaration name: (identifier) @name) @definition.function

; ── Exported const arrow / function expressions → Function ─
; These MUST come before the non-exported patterns so they win on priority
; when the same @name node matches both.
(export_statement
  (lexical_declaration
    (variable_declarator
      name: (identifier) @name
      value: (arrow_function)))) @definition.function

(export_statement
  (lexical_declaration
    (variable_declarator
      name: (identifier) @name
      value: (function_expression)))) @definition.function

(export_statement
  (lexical_declaration
    (variable_declarator
      name: (identifier) @name
      value: (generator_function)))) @definition.function

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

; ── Const values → Constant ���─────────────��────────────────
(lexical_declaration
  (variable_declarator
    name: (identifier) @name) @definition.constant)

; NOTE: interface_declaration and type_alias_declaration are TypeScript-only
; and do not exist in the plain JavaScript grammar.
