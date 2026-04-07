; language: gleam
; extensions: gleam
; grammar: gleam

; ── Doc comments (// and /// style) ─────────────────────
; The engine associates standalone @doc captures with the next symbol.
(comment) @doc
(statement_comment) @doc

; ── Custom type definitions (pub type Color { Red Green Blue }) ──
(type_definition
  (type_name (type_identifier) @name)) @definition.type

; ── Type aliases (pub type Alias = String) ───────────────
(type_alias
  (type_name (type_identifier) @name)) @definition.type

; ── Constants (pub const max_size: Int = 100) ────────────
(constant
  name: (identifier) @name) @definition.constant

; ── Functions (pub fn greet / fn helper) ─────────────────
(function
  name: (identifier) @name) @definition.function
