; Python import captures.
; @specifier — module path
; @name — each imported name (omitted for bare `import X` / wildcard)

; from X import A, B
(import_from_statement
  module_name: (dotted_name) @specifier
  name: (dotted_name) @name)

; from X import A as alias
(import_from_statement
  module_name: (dotted_name) @specifier
  name: (aliased_import name: (dotted_name) @name))

; from X import *
(import_from_statement
  module_name: (dotted_name) @specifier
  (wildcard_import))

; from .X import A, B / from . import A
(import_from_statement
  module_name: (relative_import) @specifier
  name: (dotted_name) @name)

; from .X import A as alias
(import_from_statement
  module_name: (relative_import) @specifier
  name: (aliased_import name: (dotted_name) @name))

; from .X import * / from . import *
(import_from_statement
  module_name: (relative_import) @specifier
  (wildcard_import))

; import X
(import_statement
  name: (dotted_name) @specifier)

; import X as Y
(import_statement
  name: (aliased_import name: (dotted_name) @specifier))
