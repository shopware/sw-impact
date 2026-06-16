; Likely run the ones below explicitly on vue option api object

; Emits
(object
  (pair
    key: (property_identifier) @_emits.key
    value: (array
             (_
               (string_fragment) @emit
             )
           )
    (#eq? @_emits.key "emits")
  )
)

; Props
(object
  (pair
    key: (property_identifier) @_vue.props
    value: (_ 
           ) @props.value
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

; TODO: future idea: maybe scan for TS type definitions as well? only publicly used ones would be interesting though...

