; Rasmalai local scope tracking queries

(block) @local.scope
(class_body) @local.scope
(trait_body) @local.scope
(interface_body) @local.scope
(extension_body) @local.scope

(function_declaration
  name: (identifier) @local.definition)

(parameter
  name: (identifier) @local.definition)

(variable_declaration
  name: (identifier) @local.definition)

(identifier) @local.reference
