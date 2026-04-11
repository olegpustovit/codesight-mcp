; injections.scm — JavaScript embedded languages
;
; Tagged template literals with a known tag (`sql`...``) delegate the template body
; to the embedded language.

; ── Tagged template: sql`SELECT ...` → SQL ────────────────
((call_expression
   function: (identifier) @_tag
   arguments: (template_string) @injection.content)
 (#eq? @_tag "sql")
 (#set! injection.language "sql"))
