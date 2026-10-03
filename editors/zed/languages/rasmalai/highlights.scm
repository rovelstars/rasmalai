; Rasmalai syntax highlighting queries

(module_comment) @comment.documentation
(doc_comment) @comment.documentation
(doc_tag) @attribute
(doc_star) @comment.documentation
(line_comment) @comment
(block_comment) @comment

(string_literal) @string
(interpolation) @string.special
(number_literal) @number
(boolean_literal) @boolean
(string_literal) @string
(interpolation) @string.special
(number_literal) @number
(boolean_literal) @boolean

[
  "pub"
  "public"
  "private"
  "static"
  "fn"
  "let"
  "const"
  "class"
  "struct"
  "record"
  "trait"
  "enum"
  "with"
  "extends"
  "native"
  "export"
] @keyword

[
  "if"
  "else"
  "while"
  "for"
  "in"
  "switch"
  "case"
  "default"
  "return"
  "break"
  "continue"
] @keyword.control

[
  "async"
  "await"
  "defer"
  "throw"
  "throws"
  "try"
  "catch"
  "finally"
  "guard"
  "do"
  "is"
] @keyword.control

"as" @keyword.control

[
  "test"
  "bench"
] @keyword.special

"=>" @operator

(arrow_function
  parameter: (identifier) @variable.parameter)

(primitive_type) @type.builtin
(simd_type) @type.builtin
(generic_type) @type

(function_declaration
  name: (identifier) @function)

(method_declaration
  name: (identifier) @function)

(function_signature
  name: (identifier) @function)

(init_declaration) @function
(deinit_declaration) @function
(onreload_declaration) @function

(import_declaration
  "import" @keyword.control.import
  "from" @keyword.control.import)

(export_declaration
  "export" @keyword.control.import
  "from" @keyword.control.import)

(import_source
  "native" @keyword.control.import)

(native_declaration
  "native" @keyword.control.import)

(call_expression
  function: (identifier) @function.call)

(member_expression
  member: (identifier) @property)

(call_expression
  function: (member_expression
    member: (identifier) @function.method))

(class_declaration
  name: (identifier) @type)

(struct_declaration
  name: (identifier) @type)

(trait_declaration
  name: (identifier) @type)

(enum_declaration
  name: (identifier) @type)

(record_declaration
  name: (identifier) @type)

(type
  (identifier) @type)

(parameter
  name: (identifier) @variable.parameter)

(variable_declaration
  name: (identifier) @variable)

(this_expression) @variable.builtin
