; language: json
; extensions: json
; grammar: json

; ── Top-level object keys ─────────────────────────────────
; JSON extractor captures keys from the root object's pairs.
; tree-sitter-json: document > object > pair (key, value)
(document (object
  (pair key: (string (string_content) @name)) @definition.constant))
