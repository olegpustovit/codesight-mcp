; language: kotlin
; extensions: kt kts
; grammar: kotlin

; ── Classes (including data classes, enum classes, sealed classes) ──
(class_declaration
  "class"
  name: (identifier) @name) @definition.class

; ── Interfaces ───────────────────────────────────────────────
(class_declaration
  "interface"
  name: (identifier) @name) @definition.type

; ── Object declarations ──────────────────────────────────────
(object_declaration
  name: (identifier) @name) @definition.class

; ── Companion objects (scope only — members are scoped inside) ─
(companion_object
  name: (identifier) @name) @definition.class.scope_only

; ── Methods (functions inside class bodies) ──────────────────
; Pattern order matters: method before function so class-contained
; functions are captured as methods via containment scoping.
(class_body
  (function_declaration
    name: (identifier) @name) @definition.method)

; ── Top-level functions ──────────────────────────────────────
(function_declaration
  name: (identifier) @name) @definition.function

; ── Type aliases ─────────────────────────────────────────────
(type_alias
  type: (identifier) @name) @definition.type

; ── Constants (top-level val/var with ALL_CAPS names) ────────
(property_declaration
  (variable_declaration
    (identifier) @name
    (#match? @name "^[A-Z][A-Z0-9_]+$"))) @definition.constant.nofunc
