; language: scss
; extensions: scss
; grammar: scss

; ── Variables ────────────────────────────────────────────
(declaration
  (property_name) @name
  (#match? @name "^\\$")) @definition.constant

(declaration
  (variable) @name) @definition.constant

; ── Mixins / functions ──────────────────────────────────
(mixin_statement
  name: (identifier) @name
  (#set! name_prefix "@mixin ")) @definition.function

(function_statement
  name: (identifier) @name
  (#set! name_prefix "@function ")) @definition.function

; ── Placeholders ────────────────────────────────────────
(placeholder) @name @definition.class

; ── Selectors ───────────────────────────────────────────
(rule_set (selectors) @name) @definition.class

; ── Keyframes / @media / @supports ──────────────────────
(keyframes_statement
  (keyframes_name) @name
  (#set! name_prefix "@keyframes ")) @definition.function

(media_statement
  (feature_query) @name
  (#set! name_prefix "@media ")) @definition.type

(supports_statement
  (feature_query) @name
  (#set! name_prefix "@supports ")) @definition.type
