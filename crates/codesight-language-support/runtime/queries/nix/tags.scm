; language: nix
; extensions: nix
; grammar: nix

; ── Functions: bindings whose value is a function_expression ─
; Listed FIRST so they win dedup over the generic constant pattern below.
(binding
  attrpath: (attrpath attr: (identifier) @name)
  expression: (function_expression)) @definition.function

; ── Constants: all other bindings ────────────────────────────
; Anything not matched as a function above becomes a constant.
; Excluded inside functions to avoid capturing local let bindings.
(binding
  attrpath: (attrpath attr: (identifier) @name)) @definition.constant.nofunc
