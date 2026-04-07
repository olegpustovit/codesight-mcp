; language: css
; extensions: css
; grammar: css

; ── Selectors ─────────────────────────────────────────────
(rule_set (selectors) @name) @definition.class

; ── Keyframes ─────────────────────────────────────────────
(keyframes_statement
  (keyframes_name) @name
  (#set! name_prefix "@keyframes ")) @definition.function

; ── @media ───────────────────────────────────────────────
(media_statement
  (feature_query) @name
  (#set! name_prefix "@media ")) @definition.type

; ── @supports ────────────────────────────────────────────
(supports_statement
  (feature_query) @name
  (#set! name_prefix "@supports ")) @definition.type
