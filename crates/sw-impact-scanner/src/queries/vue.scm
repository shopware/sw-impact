; top level file queries
;(program 
;  (comment) @vue.toplevel.comment
;)
;
;; export default {...}
;(program
;  (export_statement
;    value: (object)
;  ) @vue.component
;)
;
;; export default Component.wrapComponentConfig({
;(program 
;  (export_statement
;    value: (call_expression
;      arguments: (arguments
;        (object) @vue.component
;      )
;    )
;  )
;)

; Likely run the ones below explicitly on vue option api object

; Vue emit
; emits: ['close', 'confirm']
(object
  (pair
    key: (property_identifier) @_emits.key
    value: (array
             (_
               (string_fragment) @emit.name
             )
           )
    (#eq? @_emits.key "emits")
  )
)

; Props
(object
  (pair
    key: (property_identifier) @_vue.props
    value: (object 
             (pair
               key: (_) @vue.prop.name
               value: (_) @vue.prop.definition
                )
           )
    (#eq? @_vue.props "props")
  )
)

; Computed
(object
  (pair
    key: (property_identifier) @_computed.key
    value: (object 
               (method_definition) @computed
           )
    (#eq? @_computed.key "computed")
  )
)

; Methods
(object
  (pair
    key: (property_identifier) @_method.key
    value: (object 
                   (method_definition) @method
           )
    (#eq? @_method.key "methods")
  )
)

