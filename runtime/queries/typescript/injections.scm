; injections.scm — TypeScript embedded languages (mirrors javascript)

; ── Tagged template: sql`SELECT ...` → SQL ────────────────
((call_expression
   function: (identifier) @_tag
   arguments: (template_string) @injection.content)
 (#eq? @_tag "sql")
 (#set! injection.language "sql"))
