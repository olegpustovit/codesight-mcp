; language: erlang
; extensions: erl hrl
; grammar: erlang

; ── Functions ────────────────────────────────────────────
; Each fun_decl captures a function (or one clause of a multi-clause function).
; Multi-clause functions (same name, separated by ;) will produce one symbol per clause;
; the first clause establishes the name.
(fun_decl
  clause: (function_clause
    name: (atom) @name)) @definition.function

; ── Types (-type Name() :: ...) ──────────────────────────
(type_alias
  name: (type_name
    name: (atom) @name)) @definition.type

; ── Records (-record(name, {...})) ───────────────────────
(record_decl
  name: (atom) @name) @definition.type

; ── Macros/defines (-define(NAME, value)) ────────────────
(pp_define
  lhs: (macro_lhs
    name: (var) @name)) @definition.constant
