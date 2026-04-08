; language: swift
; extensions: swift
; grammar: swift

; ── Classes, Structs, Enums, Actors ──────────────────────
; tree-sitter-swift uses class_declaration for class/struct/enum/actor.
; When the name is a plain type_identifier (struct Foo, class Foo, etc.):
(class_declaration name: (type_identifier) @name) @definition.class

; ── Extensions ───────────────────────────────────────────
; Extensions are also class_declaration, but the name field wraps the
; type_identifier in user_type: `extension MyStruct { ... }`.
(class_declaration name: (user_type (type_identifier) @name)) @definition.class

; ── Protocols ─────────────────────────────────────────────
(protocol_declaration name: (type_identifier) @name) @definition.type

; ── Typealiases ───────────────────────────────────────────
(typealias_declaration name: (type_identifier) @name) @definition.type

; ── ALL_CAPS constants (let GLOBAL_CONST = 42) ───────────
(property_declaration
  name: (pattern
    bound_identifier: (simple_identifier) @name)
  (#match? @name "^[A-Z][A-Z0-9_]+$")) @definition.constant

; ── Properties (var body, @State var count, etc.) ─────────
; Lower priority than ALL_CAPS constants above (pattern order).
(property_declaration
  name: (pattern
    bound_identifier: (simple_identifier) @name)) @definition.constant

; ── Methods & Functions ───────────────────────────────────
(function_declaration name: (simple_identifier) @name) @definition.function

; ── Initializers ──────────────────────────────────────────
; NOTE: init_declaration has no name field; engine uses fixed_name from #set!
; Since #set! can cause SIGABRT on some platforms, we skip init for now.
; TODO: Add init_declaration support via capture naming convention.
