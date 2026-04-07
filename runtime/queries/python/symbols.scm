; language: python
; extensions: py
; grammar: python

; ── Classes ───────────────────────────────────────────────
(class_definition name: (identifier) @name) @definition.class

(decorated_definition
  (class_definition name: (identifier) @name)) @definition.class

; ── Methods (inside class body — listed BEFORE generic functions so they win dedup) ──
(class_definition
  body: (block
    (function_definition name: (identifier) @name) @definition.method))

(class_definition
  body: (block
    (decorated_definition
      (function_definition name: (identifier) @name)) @definition.method))

; ── Functions ─────────────────────────────────────────────
(function_definition name: (identifier) @name) @definition.function

(decorated_definition
  (function_definition name: (identifier) @name)) @definition.function

; ── Constants (ALL_CAPS assignments, excluded inside functions) ──
(expression_statement
  (assignment left: (identifier) @name
    (#match? @name "^[A-Z][A-Z0-9_]+$"))) @definition.constant.nofunc

; ── Docstrings ────────────────────────────────────────────
(function_definition
  body: (block . (expression_statement (string) @doc)))

(class_definition
  body: (block . (expression_statement (string) @doc)))
