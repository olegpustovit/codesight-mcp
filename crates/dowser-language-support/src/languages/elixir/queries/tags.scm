; language: elixir
; extensions: ex exs
; grammar: elixir

; ── Modules (defmodule) ──────────────────────────────────
(call
  target: (identifier) @_kw
  (arguments (alias) @name)
  (#eq? @_kw "defmodule")) @definition.class

; ── Protocol implementations (defimpl — must precede defprotocol for priority) ──
(call
  target: (identifier) @_kw
  (arguments (alias) @name)
  (#eq? @_kw "defimpl")) @definition.class

; ── Protocols (defprotocol) ──────────────────────────────
(call
  target: (identifier) @_kw
  (arguments (alias) @name)
  (#eq? @_kw "defprotocol")) @definition.type

; ── Functions: def with simple call arg ──────────────────
(call
  target: (identifier) @_kw
  (arguments (call target: (identifier) @name))
  (#match? @_kw "^(def|defp|defmacro|defguard)$")) @definition.method

; ── Functions: def with when guard (binary_operator wrapping call) ──
(call
  target: (identifier) @_kw
  (arguments (binary_operator left: (call target: (identifier) @name)))
  (#match? @_kw "^(def|defp|defmacro|defguard)$")) @definition.method

; ── Type aliases (@type name :: ...) ─────────────────────
(unary_operator
  operator: "@"
  operand: (call
    target: (identifier) @_attr
    (arguments
      (binary_operator
        left: (identifier) @name))
    (#eq? @_attr "type"))) @definition.type

; ── Callbacks (@callback name(...) :: ...) ───────────────
(unary_operator
  operator: "@"
  operand: (call
    target: (identifier) @_attr
    (arguments
      (binary_operator
        left: (call
          target: (identifier) @name)))
    (#eq? @_attr "callback"))) @definition.type

; ── Docstrings (@moduledoc """ ... """) ──────────────────
(unary_operator
  operand: (call
    target: (identifier) @_attr
    (arguments (string) @doc)
    (#eq? @_attr "moduledoc")))

; ── Docstrings (@doc """ ... """) ────────────────────────
(unary_operator
  operand: (call
    target: (identifier) @_attr
    (arguments (string) @doc)
    (#eq? @_attr "doc")))
