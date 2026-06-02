; top level file queries
(program 
  (comment) @vue.toplevel.comment
)

; export default {...}
(program
  (export_statement
    value: (object)
  ) @vue.component
)

; export default Component.wrapComponentConfig({
(program 
  (export_statement
    value: (call_expression
      arguments: (arguments
        (object) @vue.component
      )
    )
  )
)

; Likely run the ones below explicitly on vue option api object

; Vue emit
; emits: ['close', 'confirm']
(object
  (pair
    key: (property_identifier) @_vue.emits.key
    value: (array
             (_
               (string_fragment) @vue.emit.name
             )
           )
    (#eq? @_vue.emits.key "emits")
  )
)

; Computed
(object
  (pair
    key: (property_identifier) @_vue.computed.key
    value: (object 
             (method_definition
                name: (_) @vue.computed.name
                parameters: (_) @vue.computed.parameters
              )
           )
    (#eq? @_vue.computed.key "computed")
  )
)

; Methods
(object
  (pair
    key: (property_identifier) @_vue.method.key
    value: (object 
             (method_definition
                name: (_) @vue.method.name
                parameters: (_) @vue.method.parameters
              )
           )
    (#eq? @_vue.computed.key "methods")
  )
)

; TODO: props

