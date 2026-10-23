; language: ejs
; extensions: ejs
; grammar: embedded_template
; override_extractor: ejs
;
; Host-level captures for EJS are minimal — `code` nodes are delegated to JavaScript
; via `injections.scm`. The residual `EjsExtractor` override supplies the synthetic
; template symbol and scans `include(...)` directives as Import symbols (host
; semantics tree-sitter-embedded-template does not encode structurally).
;
; Exit criteria for the override: once tree-sitter-embedded-template (or a tagged
; replacement) exposes `include` calls and raw-output context as distinct node kinds,
; both post-passes can move into this file as structural captures.

; ── Directives as scoping anchors (no host symbols emitted by the pack) ─
(directive) @definition.module.scope_only
