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
(null_literal) @constant.builtin

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
  "interface"
  "extension"
  "enum"
  "with"
  "extends"
  "native"
  "runtime"
  "decay"
  "export"
  "new"
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
  "pass"
  "fallthrough"
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
  "unsafe"
] @keyword.control

"as" @keyword.control

[
  "test"
  "bench"
] @keyword.special

(attribute
  "#" @punctuation.special
  name: (identifier) @attribute)

(attribute
  "[" @punctuation.bracket)

(attribute
  "]" @punctuation.bracket)

(macro_expression
  macro: (identifier) @function.macro)

(arrow_function
  parameter: (identifier) @variable.parameter)

(primitive_type) @type.builtin
(simd_type) @type.builtin
(generic_type) @type
(path_type) @type
(fn_type) @type
(tuple_type) @type

(function_declaration
  name: (identifier) @function)

(method_declaration
  name: (identifier) @function)

(function_signature
  name: (identifier) @function)

(init_declaration
  "init" @function)

(deinit_declaration
  "deinit" @function)

(onreload_declaration
  "onReload" @function)

(import_declaration
  "import" @keyword.control.import
  "from" @keyword.control.import)

(export_declaration
  "export" @keyword.control.import
  "from" @keyword.control.import)

(import_source
  "native" @keyword.control.import)

(import_source
  "runtime" @keyword.control.import)

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

(interface_declaration
  name: (identifier) @type)

(enum_declaration
  name: (identifier) @type)

(record_declaration
  name: (identifier) @type)

(enum_variant
  name: (identifier) @constructor)

(implicit_member_expression
  member: (identifier) @constructor)

(record_field
  name: (identifier) @property)

(record_field
  name: (string_literal) @property)

(argument
  name: (identifier) @variable.parameter)

(type
  (identifier) @type)

(parameter
  name: (identifier) @variable.parameter)

(variable_declaration
  name: (identifier) @variable)

(destructure_tuple_statement
  (identifier) @variable)

(destructure_array_statement
  (identifier) @variable)

(destructure_record_statement
  (identifier) @variable)

(this_expression) @variable.builtin
(super_expression) @variable.builtin

[
  "=>"
  "??"
  "?."
  ".."
  "..="
  "..."
  "=="
  "!="
  "<="
  ">="
  "&&"
  "||"
  "<<"
  ">>"
  ">>>"
  "="
  "+="
  "-="
  "*="
  "/="
  "%="
  "<<="
  ">>="
  ">>>="
  "++"
  "--"
  "+"
  "-"
  "*"
  "/"
  "%"
  "!"
  "~"
  "&"
  "|"
  "^"
  "?"
] @operator

[
  "("
  ")"
  "{"
  "}"
  "["
  "]"
] @punctuation.bracket

[
  ","
  ";"
  ":"
  "."
] @punctuation.delimiter
