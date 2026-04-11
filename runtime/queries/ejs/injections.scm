; injections.scm — EJS embedded JavaScript
;
; `<% ... %>`, `<%= ... %>`, and `<%- ... %>` directives all wrap their code under a
; `(code)` node in tree-sitter-embedded-template. The injection phase delegates each
; span to the JavaScript extractor and remaps resulting symbols back into host
; coordinates via `containers::extract_host_segment`.

; EJS scriptlets split one JS program across many `<% %>` boundaries. Combined mode
; tells the injection runner to gather every `(code)` span into one masked host-sized
; buffer before delegating, so a function declared in one directive and closed in
; another still parses as a single JS AST.
((code) @injection.content
 (#set! injection.language "javascript")
 (#set! injection.combined "true"))
