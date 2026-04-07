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
(impl_item type: (type_identifier) @name) @definition.class.scope_only

; ── Functions & Methods ───────────────────────────────────
(function_item name: (identifier) @name) @definition.function

; ── Constants & Statics ───────────────────────────────────
(const_item name: (identifier) @name) @definition.constant
(static_item name: (identifier) @name) @definition.constant
