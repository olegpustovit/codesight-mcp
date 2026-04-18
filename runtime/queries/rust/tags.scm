; language: rust
; extensions: rs
; grammar: rust

; ── Modules (scope only) ──────────────────────────────────
(mod_item name: (identifier) @name) @definition.module.scope_only

; ── Structs & Enums ───────────────────────────────────────
(struct_item name: (type_identifier) @name) @definition.class
(enum_item name: (type_identifier) @name) @definition.type

; ── Traits ────────────────────────────────────────────────
(trait_item name: (type_identifier) @name) @definition.type

; ── Type aliases ──────────────────────────────────────────
(type_item name: (type_identifier) @name) @definition.type

; ── Impl blocks (scope only — scope label from type) ──────
; Plain `impl Foo { … }` and `impl Trait for Foo { … }`.
(impl_item type: (type_identifier) @name) @definition.class.scope_only
; Generic `impl<T> Foo<T> { … }` / `impl<T> Trait for Foo<T> { … }`.
; Without this, methods inside generic impls lose their receiver prefix and
; collide with same-named methods from other impls in the same file.
(impl_item type: (generic_type type: (type_identifier) @name)) @definition.class.scope_only

; ── Functions & Methods ───────────────────────────────────
(function_item name: (identifier) @name) @definition.function

; ── Public reexports ──────────────────────────────────────
(use_declaration
  (visibility_modifier)
  argument: (identifier) @name) @definition.reexport

(use_declaration
  (visibility_modifier)
  argument: (scoped_identifier) @name) @definition.reexport

; ── Constants & Statics ───────────────────────────────────
(const_item name: (identifier) @name) @definition.constant
(static_item name: (identifier) @name) @definition.constant
