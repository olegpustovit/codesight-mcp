; language: csharp
; extensions: cs
; grammar: c_sharp

; ── Classes & Structs ─────────────────────────────────────
(class_declaration name: (identifier) @name) @definition.class
(struct_declaration name: (identifier) @name) @definition.class
(record_declaration name: (identifier) @name) @definition.class
; NOTE: record_struct_declaration does not exist in tree-sitter-c-sharp 0.23.1

; ── Interfaces & Enums ────────────────────────────────────
(interface_declaration name: (identifier) @name) @definition.type
(enum_declaration name: (identifier) @name) @definition.type

; ── Methods & Constructors ────────────────────────────────
(method_declaration name: (identifier) @name) @definition.method
(constructor_declaration name: (identifier) @name) @definition.method

; ── Properties & fields ───────────────────────────────────
(property_declaration name: (identifier) @name) @definition.constant

(field_declaration
  (variable_declaration
    (variable_declarator name: (identifier) @name))) @definition.constant
