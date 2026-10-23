; language: perl
; extensions: pl pm t
; grammar: perl

; ── Packages (class-like) ────────────────────────────────
(package_statement
  (package_name (identifier) @name)) @definition.class

; ── Functions ────────────────────────────────────────────
(function_definition
  name: (identifier) @name) @definition.function

; ── Constants (use constant NAME => value) ───────────────
(use_constant_statement
  constant: (identifier) @name) @definition.constant
