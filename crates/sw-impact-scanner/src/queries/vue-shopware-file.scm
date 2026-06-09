; top level file queries
(program 
  (comment) @toplevel.comment
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
      arguments: (arguments
        (object) @component
      )
    )
  )
)
