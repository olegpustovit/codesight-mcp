; language: dart
; extensions: dart
; grammar: dart

; ── Classes ──────────────────────────────────────────────
(class_declaration name: (identifier) @name) @definition.class

; ── Mixins ───────────────────────────────────────────────
(mixin_declaration name: (identifier) @name) @definition.class

; ── Extensions ───────────────────────────────────────────
(extension_declaration name: (identifier) @name) @definition.class

; ── Enums ────────────────────────────────────────────────
(enum_declaration name: (identifier) @name) @definition.type

; ── Typedefs ─────────────────────────────────────────────
(type_alias (type_identifier) @name) @definition.type

; ── Methods (function signatures inside class/mixin/etc) ──
((method_signature
  (function_signature name: (identifier) @name)) @definition.method
  . (function_body)? @definition.method)

; ── Getters inside classes ───────────────────────────────
((method_signature
  (getter_signature name: (identifier) @name)) @definition.method
  . (function_body)? @definition.method)

; ── Setters inside classes ───────────────────────────────
((method_signature
  (setter_signature name: (identifier) @name)) @definition.method
  . (function_body)? @definition.method)

; ── Constructors ─────────────────────────────────────────
((constructor_signature name: (identifier) @name) @definition.method
  . (function_body)? @definition.method)

; ── Top-level functions ──────────────────────────────────
((function_signature name: (identifier) @name) @definition.function
  . (function_body)? @definition.function)

; ── Top-level getters ────────────────────────────────────
((getter_signature name: (identifier) @name) @definition.function
  . (function_body)? @definition.function)

; ── Top-level setters ────────────────────────────────────
((setter_signature name: (identifier) @name) @definition.function
  . (function_body)? @definition.function)
