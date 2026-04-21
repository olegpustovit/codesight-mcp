; language: php
; extensions: php
; grammar: php

; ── Namespaces (scope only) ───────────────────────────────
(namespace_definition name: (namespace_name) @name) @definition.namespace.scope_only

; ── Classes ───────────────────────────────────────────────
(class_declaration name: (name) @name) @definition.class

; ── Interfaces, Traits, Enums ─────────────────────────────
(interface_declaration name: (name) @name) @definition.type
(trait_declaration name: (name) @name) @definition.type
(enum_declaration name: (name) @name) @definition.type

; ── Methods ───────────────────────────────────────────────
(method_declaration name: (name) @name) @definition.method

; ── Functions ─────────────────────────────────────────────
(function_definition name: (name) @name) @definition.function

; ── Constants ─────────────────────────────────────────────
(const_declaration
  (const_element (name) @name)) @definition.constant
