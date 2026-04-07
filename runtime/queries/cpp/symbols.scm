; language: cpp
; extensions: c cc cpp cxx h hh hpp hxx inl ipp tpp
; grammar: cpp

; ── Namespaces (scope only) ───────────────────────────────
(namespace_definition name: (namespace_identifier) @name) @definition.namespace.scope_only

; Anonymous namespace
(namespace_definition !name) @definition.namespace.scope_only

; ── Classes & Structs ─────────────────────────────────────
(class_specifier name: (type_identifier) @name) @definition.class
(struct_specifier name: (type_identifier) @name) @definition.class

; ── Enums ─────────────────────────────────────────────────
(enum_specifier name: (type_identifier) @name) @definition.type

; ── Functions — enumerate declarator wrapping patterns ─────
; Plain function
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @name)) @definition.function

; Qualified method (Class::method)
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier) @name)) @definition.function

; Pointer function (pointer in declarator)
(function_definition
  declarator: (function_declarator
    declarator: (pointer_declarator
      declarator: (identifier) @name))) @definition.function

; Pointer-returning function (pointer wraps function_declarator, e.g. struct Foo *func())
(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (identifier) @name))) @definition.function

; Reference function (reference_declarator has no 'declarator' field)
(function_definition
  declarator: (function_declarator
    declarator: (reference_declarator
      (identifier) @name))) @definition.function

; Operator overload
(function_definition
  declarator: (function_declarator
    declarator: (operator_name) @name)) @definition.function

; Conversion operator (e.g. operator bool())
(function_definition
  declarator: (operator_cast) @name) @definition.function

; Complex declarator: function-pointer-returning function
; e.g. int (*make_callback(int seed))(int) { ... }
(function_definition
  declarator: (function_declarator
    declarator: (parenthesized_declarator
      (pointer_declarator
        declarator: (function_declarator
          declarator: (identifier) @name))))) @definition.function

; Destructor
(function_definition
  declarator: (function_declarator
    declarator: (destructor_name) @name)) @definition.function

; Template functions
(template_declaration
  (function_definition
    declarator: (function_declarator
      declarator: (identifier) @name))) @definition.function

(template_declaration
  (function_definition
    declarator: (function_declarator
      declarator: (qualified_identifier) @name))) @definition.function
