; top level file queries
(program 
  (comment) @toplevel.comment
)

; import template from './sw-cms-el-form-contact.html.twig';
; matches all import string_fragments
(program
  (import_statement
    source: (string
              (string_fragment) @import
    )
  )
)

; export default {...}
(program
  (export_statement
    value: (object)
  ) @component
)

; export default Component.wrapComponentConfig({
(program 
  (export_statement
    value: (call_expression
             function: (_) @_fn
             arguments: (arguments
               (object) @component
             )
             (#match? @_fn "wrapComponentConfig")
    )
  )
)
