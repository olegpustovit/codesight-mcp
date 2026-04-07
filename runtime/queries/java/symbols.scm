; language: java
; extensions: java
; grammar: java

; ── Classes ───────────────────────────────────────────────
(class_declaration name: (identifier) @name) @definition.class

; ── Interfaces & Enums ────────────────────────────────────
(interface_declaration name: (identifier) @name) @definition.type
(enum_declaration name: (identifier) @name) @definition.type

; ── Methods ───────────────────────────────────────────────
(method_declaration name: (identifier) @name) @definition.method
(constructor_declaration name: (identifier) @name) @definition.method

; ── Constants (ALL_CAPS fields) ───────────────────────────
(field_declaration
  (variable_declarator name: (identifier) @name
    (#match? @name "^[A-Z][A-Z0-9_]+$"))) @definition.constant
