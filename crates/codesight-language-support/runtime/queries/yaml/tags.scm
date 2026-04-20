; language: yaml
; extensions: yaml yml
; grammar: yaml
; override_extractor: yaml

; ── Nested mapping keys → Type ───────────────────────────
(block_mapping_pair
  key: (_) @name
  value: (block_node)) @definition.type

; ── All other mapping keys → Constant ────────────────────
(block_mapping_pair
  key: (_) @name) @definition.constant
