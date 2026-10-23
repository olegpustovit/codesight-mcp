; JavaScript import captures.
; @specifier — import path string literal
; @name — each imported identifier

; import "mod" (side-effect only)
(import_statement
  source: (string) @specifier)

; import X from "mod"   (default binding)
; import * as X from "mod"   (namespace)
(import_statement
  (import_clause
    (identifier) @name)
  source: (string) @specifier)

(import_statement
  (import_clause
    (namespace_import (identifier) @name))
  source: (string) @specifier)

; import { A, B as C } from "mod"
(import_statement
  (import_clause
    (named_imports
      (import_specifier name: (identifier) @name)))
  source: (string) @specifier)

; export { A } from "mod"
(export_statement
  (export_clause
    (export_specifier name: (identifier) @name))
  source: (string) @specifier)

; export * from "mod"
(export_statement
  source: (string) @specifier)

; require("mod")
(call_expression
  function: (identifier) @_fn
  arguments: (arguments (string) @specifier)
  (#eq? @_fn "require"))

; dynamic import("mod")
(call_expression
  function: (import)
  arguments: (arguments (string) @specifier))
