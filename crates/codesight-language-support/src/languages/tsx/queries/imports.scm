; TSX import captures — identical to TypeScript.

(import_statement
  source: (string) @specifier)

(import_statement
  (import_clause
    (identifier) @name)
  source: (string) @specifier)

(import_statement
  (import_clause
    (namespace_import (identifier) @name))
  source: (string) @specifier)

(import_statement
  (import_clause
    (named_imports
      (import_specifier name: (identifier) @name)))
  source: (string) @specifier)

(export_statement
  (export_clause
    (export_specifier name: (identifier) @name))
  source: (string) @specifier)

(export_statement
  source: (string) @specifier)

(call_expression
  function: (identifier) @_fn
  arguments: (arguments (string) @specifier)
  (#eq? @_fn "require"))

(call_expression
  function: (import)
  arguments: (arguments (string) @specifier))
