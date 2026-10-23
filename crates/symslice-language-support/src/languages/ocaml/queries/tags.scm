; language: ocaml
; extensions: ml
; grammar: ocaml
; Definition patterns adapted from tree-sitter-ocaml 0.26.0 (MIT), queries/tags.scm.
; Interface value specifications added; reference captures are intentionally omitted.
(comment) @doc
(module_definition (module_binding (module_name) @name) @definition.module)
(module_type_definition (module_type_name) @name) @definition.type
(class_definition (class_binding (class_name) @name) @definition.class)
(class_type_definition (class_type_binding (class_type_name) @name) @definition.class)
(method_definition (method_name) @name) @definition.method
(type_definition (type_binding name: [(type_constructor) @name
    (type_constructor_path (type_constructor) @name)]) @definition.type)
(value_definition [
    (let_binding pattern: (value_name) @name (parameter))
    (let_binding pattern: (value_name) @name body: [(fun_expression) (function_expression)])
] @definition.function)
(external (value_name) @name) @definition.function
(value_specification (value_name) @name type: (function_type)) @definition.function
