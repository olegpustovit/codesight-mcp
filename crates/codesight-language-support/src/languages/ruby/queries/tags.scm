; language: ruby
; extensions: rb
; grammar: ruby

; ── Classes & Modules ─────────────────────────────────────
(class name: (constant) @name) @definition.class
(class name: (scope_resolution) @name) @definition.class
(module name: (constant) @name) @definition.class
(module name: (scope_resolution) @name) @definition.class

; ── Functions (top-level methods — must precede generic method for dedup priority) ──
(program (method name: (identifier) @name) @definition.function)

; ── Methods ───────────────────────────────────────────────
(method name: (identifier) @name) @definition.method

; ── Singleton methods (self.method) ───────────────────────
(singleton_method name: (identifier) @name) @definition.method
