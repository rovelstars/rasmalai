function commaSep(rule) {
  return optional(commaSep1(rule));
}

function commaSep1(rule) {
  return seq(rule, repeat(seq(',', rule)));
}

// Shared expression alternatives. `full` includes the trailing-closure call
// (`Name { ... }`); condition positions (`if`/`while`/`for..in`/`switch`/guards)
// exclude it because the real parser parses conditions with brace_ok=false
// (compiler/frontend/src/parser.rs cond(), parser.rs:2569), so `if x { ... }`
// always treats the block as the body, never as trailing call arguments.
function exprAlternatives($, full) {
  const list = [
    $.ternary_expression,
    $.nullish_expression,
    $.logical_or_expression,
    $.logical_and_expression,
    $.bitwise_or_expression,
    $.bitwise_xor_expression,
    $.bitwise_and_expression,
    $.equality_expression,
    $.is_expression,
    $.relational_expression,
    $.shift_expression,
    $.range_expression,
    $.additive_expression,
    $.multiplicative_expression,
    $.cast_expression,
    $.unary_expression,
    $.prefix_update_expression,
    $.postfix_update_expression,
    $.call_expression,
    $.new_expression,
    $.optional_call_expression,
    $.optional_chain_expression,
    $.member_expression,
    $.tuple_index_expression,
    $.index_expression,
    $.parenthesized_expression,
    $.tuple_expression,
    $.array_literal,
    $.record_literal,
    $.switch_expression,
    $.closure_expression,
    $.arrow_function,
    $.macro_expression,
    $.implicit_member_expression,
    $.unsafe_block,
    $.identifier,
    $.this_expression,
    $.super_expression,
    $.number_literal,
    $.boolean_literal,
    $.null_literal,
    $.string_literal,
  ];
  if (full) {
    list.push($.trailing_closure_expression);
  }
  return choice(...list);
}

// Core type without nullability. Kept in one function so the nullable and
// plain alternatives of `type` cannot drift apart.
function typeCore($) {
  return choice(
    $.primitive_type,
    $.simd_type,
    $.generic_type,
    $.path_type,
    $.fn_type,
    $.tuple_type,
    $.identifier,
  );
}

// INVARIANTS (do not break without re-verifying the full .rnx corpus):
// 1. source_file must stay the first rule: tree-sitter uses it as the start
//    rule, and anything else first silently breaks all declarations.
// 2. Conditions use _condition (no trailing-closure call): `if x {` conditions
//    and `Column { }` UI-DSL calls are token-identical, and LR cannot tell them
//    apart. The real parser runs conditions with brace_ok=false (parser.rs
//    cond()); _condition is the grammar-level equivalent. Expression
//    statements keep the trailing-closure form; both readings are declared
//    conflicts so GLR keeps them and error-cost picks correctly.
// 3. call_expression keeps the turbofish alternative at zero precedence with
//    [expression, call_expression] declared: this is the GLR version of the
//    real parser's angle_call_ahead lookahead, and it is what lets `a < b`
//    (relational) and `f<T>(x)` (generic call) coexist. Do not merge the
//    alternatives or add precedence to the turbofish branch.
// 4. `type` keeps the greedy nullable branch (prec.right(6)): the real ty()
//    eats trailing `?` before any ternary is considered, so `x is Int?`
//    must prefer nullable. As a consequence `x is T ? a : b` is an error
//    here exactly as in the real parser (parenthesize the test).
// 5. String content is token(prec(1, ...)): longest-match lexing would
//    otherwise let `//` and `/*` inside a string win as comments. Bare `}`
//    is content (only `{` opens interpolation); `\u{...}` is one escape.
// 6. Known gap: postfix `?` (Propagate) is dropped. LR cannot do the real
//    parser's colon lookahead, and keeping it misparsed every ternary.
//    Nothing in the repo uses it.

module.exports = grammar({
  name: 'rasmalai',

  extras: $ => [/\s/, $.line_comment, $.block_comment, $.doc_comment, $.module_comment],

  word: $ => $.identifier,

  conflicts: $ => [
    [$.return_statement],
    [$.throw_statement],
    [$.expression, $.statement],
    [$.expression, $._block_or_statement],
    [$.expression, $.parameter],
    [$.expression, $.parameter, $.type],
    [$.expression, $.type],
    [$.expression, $.generic_type],
    [$.expression, $.call_expression],
    [$.expression, $.call_expression, $.generic_type],
    [$.expression, $.trailing_closure_expression],
    [$._condition, $.trailing_closure_expression],
    [$.expression, $.path_type],
    [$.generic_type, $.type],
    [$.path_type, $.type],
    [$.path_type],
    [$.expression, $._condition],
    [$.tuple_expression, $.parenthesized_expression],
    [$.arrow_function, $.tuple_expression],
    [$.arrow_function, $.type],
    [$._condition, $.arrow_function],
    [$.expression, $.arrow_function],
    [$.record_literal, $.block],
    [$.switch_expression, $.switch_statement],
    [$.switch_expression_arm, $.expression_statement],
    [$.switch_expression_default, $.expression_statement],
    [$.case_clause, $.switch_expression_arm],
  ],

  rules: {
    source_file: $ => repeat($._declaration_or_statement),

    expression: $ => exprAlternatives($, true),

    _condition: $ => exprAlternatives($, false),

    _block_or_statement: $ => choice(
      $.block,
      $.statement,
      $.expression_statement,
    ),

    ternary_expression: $ => prec.right(1, seq(
      field('condition', $.expression),
      '?',
      field('consequence', $.expression),
      ':',
      field('alternative', $.expression),
    )),

    nullish_expression: $ => prec.left(2, seq($.expression, '??', $.expression)),

    logical_or_expression: $ => prec.left(3, seq($.expression, '||', $.expression)),

    logical_and_expression: $ => prec.left(4, seq($.expression, '&&', $.expression)),

    bitwise_or_expression: $ => prec.left(5, seq($.expression, '|', $.expression)),

    bitwise_xor_expression: $ => prec.left(6, seq($.expression, '^', $.expression)),

    bitwise_and_expression: $ => prec.left(7, seq($.expression, '&', $.expression)),

    equality_expression: $ => prec.left(8, seq($.expression, choice('==', '!='), $.expression)),

    is_expression: $ => prec.left(9, seq($.expression, 'is', $.type)),

    relational_expression: $ => prec.left(9, seq($.expression, choice('<', '<=', '>', '>='), $.expression)),

    shift_expression: $ => prec.left(10, seq($.expression, choice('<<', '>>', '>>>'), $.expression)),

    range_expression: $ => prec.left(11, seq($.expression, choice('..', '..='), $.expression)),

    additive_expression: $ => prec.left(12, seq($.expression, choice('+', '-'), $.expression)),

    multiplicative_expression: $ => prec.left(13, seq($.expression, choice('*', '/', '%'), $.expression)),

    cast_expression: $ => prec.left(14, seq($.expression, 'as', $.type)),

    unary_expression: $ => prec(15, seq(choice('-', '!', '~', '&', '*', 'await'), $.expression)),

    prefix_update_expression: $ => prec.right(16, seq(choice('++', '--'), $.expression)),

    postfix_update_expression: $ => prec.left(16, seq($.expression, choice('++', '--'))),

    call_expression: $ => choice(
      seq(
        field('function', choice($.identifier, $.member_expression, $.implicit_member_expression, $.call_expression, $.index_expression)),
        field('type_arguments', $.turbofish),
        $.argument_list,
      ),
      prec(17, seq(
        field('function', choice($.identifier, $.member_expression, $.implicit_member_expression, $.call_expression, $.index_expression)),
        $.argument_list,
      )),
    ),

    trailing_closure_expression: $ => seq(
      field('function', choice($.identifier, $.member_expression, $.implicit_member_expression)),
      field('trailing', $.block),
    ),

    new_expression: $ => prec(17, seq(
      'new',
      field('target', seq($.identifier, repeat(seq('.', $.identifier)))),
      optional(field('type_arguments', $.turbofish)),
      $.argument_list,
    )),

    optional_call_expression: $ => prec(18, seq(
      field('object', $.expression),
      '?.',
      field('member', $.identifier),
      $.argument_list,
    )),

    optional_chain_expression: $ => prec(17, seq(
      field('object', $.expression),
      '?.',
      field('member', $.identifier),
    )),

    closure_expression: $ => prec(9, seq(
      'fn',
      'decay',
      $.parameter_list,
      optional(seq(':', $.type)),
      optional('throws'),
      field('body', $.block),
    )),

    arrow_function: $ => prec.right(0, seq(
      optional('async'),
      choice(
        field('parameter', $.identifier),
        field('parameters', $.parameter_list)
      ),
      optional(seq(':', field('return_type', $.type))),
      '=>',
      field('body', choice($.block, $.expression))
    )),

    macro_expression: $ => prec(17, seq(
      field('macro', $.identifier),
      '!',
      choice($.argument_list, seq('[', commaSep($.expression), ']')),
    )),

    _declaration_or_statement: $ => choice(
      $._declaration,
      $.statement,
      $.expression_statement,
    ),

    _declaration: $ => choice(
      $.attribute,
      $.function_declaration,
      $.class_declaration,
      $.struct_declaration,
      $.record_declaration,
      $.trait_declaration,
      $.interface_declaration,
      $.extension_declaration,
      $.enum_declaration,
      $.test_declaration,
      $.bench_declaration,
      $.variable_declaration,
      $.import_declaration,
      $.export_declaration,
      $.native_declaration,
    ),

    _exportable_declaration: $ => choice(
      $.function_declaration,
      $.class_declaration,
      $.struct_declaration,
      $.record_declaration,
      $.trait_declaration,
      $.interface_declaration,
      $.extension_declaration,
      $.enum_declaration,
      $.variable_declaration,
    ),

    module_comment: $ => token(seq('//!', /.*/)),
    doc_comment: $ => seq(
      '/**',
      repeat(choice(
        $.doc_tag,
        '@',
        $.doc_star,
        $._doc_text,
      )),
      '*/'
    ),
    doc_tag: $ => token(seq('@', /[A-Za-z_][A-Za-z0-9_]*/)),
    doc_star: $ => '*',
    _doc_text: $ => token(prec(-1, repeat1(/[^*@]/))),
    line_comment: $ => token(seq('//', /.*/)),
    block_comment: $ => token(choice(
      '/**/',
      seq('/*', /[^*]/, /[^*]*\*+([^/*][^*]*\*+)*/, '/')
    )),

    attribute: $ => seq(
      '#',
      optional('!'),
      '[',
      field('name', $.identifier),
      optional(seq('(', commaSep($.expression), ')')),
      ']',
    ),

    function_declaration: $ => seq(
      optional(choice('pub', 'public')),
      optional(choice('async', 'unsafe')),
      'fn',
      field('name', $.identifier),
      optional($.type_parameters),
      $.parameter_list,
      optional(seq(':', $.type)),
      optional('throws'),
      field('body', $.block),
    ),

    class_declaration: $ => seq(
      optional(choice('pub', 'public')),
      'class',
      field('name', $.identifier),
      optional($.type_parameters),
      optional(seq('extends', field('superclass', $.type))),
      optional(seq('with', commaSep1($.type))),
      optional(seq(':', commaSep1($.type))),
      $.class_body,
    ),

    struct_declaration: $ => seq(
      optional(choice('pub', 'public')),
      'struct',
      field('name', $.identifier),
      optional($.type_parameters),
      $.class_body,
    ),

    record_declaration: $ => seq(
      optional(choice('pub', 'public')),
      'record',
      field('name', $.identifier),
      optional($.type_parameters),
      $.parameter_list,
    ),

    trait_declaration: $ => seq(
      optional(choice('pub', 'public')),
      'trait',
      field('name', $.identifier),
      optional($.type_parameters),
      $.trait_body,
    ),

    interface_declaration: $ => seq(
      optional(choice('pub', 'public')),
      'interface',
      field('name', $.identifier),
      optional($.type_parameters),
      $.interface_body,
    ),

    interface_body: $ => seq('{', repeat($.function_signature), '}'),

    extension_declaration: $ => seq(
      optional(choice('pub', 'public')),
      'extension',
      field('target', $.type),
      $.extension_body,
    ),

    extension_body: $ => seq('{', repeat($._class_member), '}'),

    enum_declaration: $ => seq(
      optional(choice('pub', 'public')),
      'enum',
      field('name', $.identifier),
      optional($.type_parameters),
      '{',
      seq(
        $.enum_variant,
        repeat(seq(repeat(choice(';', ',')), $.enum_variant)),
        repeat(choice(';', ',')),
      ),
      '}',
    ),

    enum_variant: $ => seq(
      optional(choice('let', 'const')),
      field('name', $.identifier),
      optional(seq('(', commaSep($.type), ')')),
    ),

    test_declaration: $ => seq(
      'test',
      'fn',
      field('name', $.identifier),
      $.parameter_list,
      $.block,
    ),

    bench_declaration: $ => seq(
      'bench',
      field('name', $.string_literal),
      $.block,
    ),

    variable_declaration: $ => seq(
      optional(choice('pub', 'public', 'private')),
      optional('static'),
      choice('let', 'const'),
      field('name', $.identifier),
      optional(seq(':', $.type)),
      optional(seq('=', $.expression)),
      optional(';'),
    ),

    destructure_tuple_statement: $ => seq(
      optional('static'),
      choice('let', 'const'),
      '(',
      commaSep1($.identifier),
      ')',
      optional(seq(':', $.type)),
      '=',
      field('value', $.expression),
      optional(';'),
    ),

    destructure_array_statement: $ => seq(
      optional('static'),
      choice('let', 'const'),
      '[',
      commaSep(choice($.identifier, seq('...', $.identifier))),
      ']',
      '=',
      field('value', $.expression),
      optional(';'),
    ),

    destructure_record_statement: $ => seq(
      optional('static'),
      choice('let', 'const'),
      '{',
      commaSep(choice(
        $.identifier,
        seq(field('field', $.identifier), ':', field('local', $.identifier)),
        seq('...', $.identifier),
      )),
      '}',
      '=',
      field('value', $.expression),
      optional(';'),
    ),

    import_declaration: $ => seq(
      'import',
      choice(
        field('source', $.import_source),
        seq(
          field('names', choice(
            seq('*', optional(seq('as', field('namespace', $.identifier)))),
            seq('{', commaSep($.import_specifier), '}'),
            seq(
              field('default', $.identifier),
              optional(seq(',', '{', commaSep($.import_specifier), '}')),
            ),
          )),
          'from',
          field('source', $.import_source),
        ),
      ),
      optional(';'),
    ),

    export_declaration: $ => seq(
      'export',
      choice(
        seq('default', $._declaration_or_statement),
        seq(
          '*',
          optional(seq('as', field('alias', $.identifier))),
          'from',
          field('source', $.import_source),
          optional(';'),
        ),
        seq(
          '{',
          commaSep($.import_specifier),
          '}',
          optional(seq('from', field('source', $.import_source))),
          optional(';'),
        ),
        field('declaration', $._exportable_declaration),
      ),
    ),

    import_source: $ => choice(
      $.string_literal,
      seq('native', field('native_library', $.string_literal)),
    ),

    import_specifier: $ => choice(
      seq(
        optional('export'),
        field('name', $.identifier),
        optional(seq('as', field('alias', $.identifier))),
      ),
      seq(
        'fn',
        field('name', $.identifier),
        $.parameter_list,
        optional(seq(':', $.type)),
        optional(seq('as', field('alias', $.identifier))),
      ),
    ),

    type_parameters: $ => seq('<', commaSep1($.identifier), '>'),

    parameter_list: $ => seq('(', commaSep($.parameter), ')'),

    parameter: $ => seq(
      optional(choice('pub', 'public', 'private')),
      field('name', choice($.identifier, $.this_expression)),
      optional(seq(':', field('type', $.type))),
      optional(seq('=', field('default', $.expression))),
    ),

    argument_list: $ => seq('(', commaSep($.argument), ')'),

    argument: $ => seq(
      optional(seq(field('name', $.identifier), ':')),
      $.expression,
    ),

    class_body: $ => seq('{', repeat($._class_member), '}'),

    _class_member: $ => choice(
      $.attribute,
      $.method_declaration,
      $.init_declaration,
      $.deinit_declaration,
      $.onreload_declaration,
      $.variable_declaration,
    ),

    trait_body: $ => seq('{', repeat($._trait_member), '}'),

    _trait_member: $ => choice(
      $.function_signature,
      $.method_declaration,
    ),

    function_signature: $ => seq(
      'fn',
      field('name', $.identifier),
      optional($.type_parameters),
      $.parameter_list,
      optional(seq(':', $.type)),
      optional(';'),
    ),

    method_declaration: $ => seq(
      optional(choice('pub', 'public', 'private')),
      optional('static'),
      optional(choice('async', 'unsafe')),
      optional('fn'),
      field('name', $.identifier),
      optional($.type_parameters),
      $.parameter_list,
      optional(seq(':', $.type)),
      optional('throws'),
      field('body', $.block),
    ),

    init_declaration: $ => seq('init', $.parameter_list, $.block),

    deinit_declaration: $ => seq('deinit', $.block),

    onreload_declaration: $ => seq('onReload', $.parameter_list, $.block),

    block: $ => seq('{', repeat($._declaration_or_statement), '}'),

    primitive_type: $ => choice('Int', 'Float', 'Bool', 'Void', 'String'),

    simd_type: $ => choice('Vec4f', 'Vec4i'),

    generic_type: $ => seq($.identifier, '<', commaSep1($.type), '>'),

    path_type: $ => seq(
      $.identifier,
      repeat1(seq('.', $.identifier)),
      optional(seq('<', commaSep1($.type), '>')),
    ),

    fn_type: $ => prec.right(2, seq(
      'fn',
      '(',
      commaSep($.type),
      ')',
      optional(seq(':', $.type)),
    )),

    tuple_type: $ => seq('(', $.type, repeat1(seq(',', $.type)), ')'),

    type: $ => choice(
      prec.right(6, seq(typeCore($), repeat1('?'))),
      typeCore($),
    ),

    guard_statement: $ => seq(
      'guard',
      'let',
      field('name', $.identifier),
      '=',
      field('value', $.expression),
      'else',
      field('body', $.block),
    ),

    native_declaration: $ => seq(
      'native',
      field('target', $.string_literal),
      '{',
      repeat($.function_signature),
      '}',
    ),

    null_literal: $ => 'null',

    record_field: $ => seq(
      field('name', choice($.identifier, $.string_literal)),
      ':',
      field('value', $.expression),
    ),

    record_literal: $ => prec(18, seq(
      '{',
      commaSep(choice($.record_field, $.spread_expression)),
      '}',
    )),

    spread_expression: $ => prec(18, seq('...', $.expression)),

    turbofish: $ => seq('<', commaSep1($.type), '>'),

    member_expression: $ => prec(17, seq(
      field('object', $.expression),
      '.',
      field('member', $.identifier),
    )),

    tuple_index_expression: $ => prec(17, seq(
      field('object', $.expression),
      '.',
      field('index', /[0-9]+/),
    )),

    index_expression: $ => prec(17, seq(
      field('object', $.expression),
      '[',
      field('index', $.expression),
      ']',
    )),

    parenthesized_expression: $ => prec(18, seq('(', $.expression, ')')),

    tuple_expression: $ => prec(18, seq('(', $.expression, repeat1(seq(',', $.expression)), ')')),

    array_literal: $ => prec(18, seq('[', commaSep(choice($.spread_expression, $.expression)), ']')),

    implicit_member_expression: $ => prec(18, seq('.', field('member', $.identifier))),

    this_expression: $ => 'this',

    super_expression: $ => 'super',

    unsafe_block: $ => seq('unsafe', field('body', $.block)),

    switch_expression: $ => seq(
      'switch',
      field('value', $._condition),
      '{',
      repeat(choice($.switch_expression_arm, $.switch_expression_default)),
      '}',
    ),

    switch_expression_arm: $ => seq(
      'case',
      field('pattern', $.pattern),
      optional(seq('if', field('guard', $._condition))),
      ':',
      field('value', choice($.block, $.expression)),
      optional(choice(',', ';')),
    ),

    switch_expression_default: $ => seq(
      'default',
      ':',
      field('value', choice($.block, $.expression)),
      optional(choice(',', ';')),
    ),

    number_literal: $ => choice(
      /0x[0-9a-fA-F_]+/,
      /0b[01_]+/,
      /0o[0-7_]+/,
      /[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9_]+)?/,
      /[0-9][0-9_]*[eE][+-]?[0-9_]+/,
      /[0-9][0-9_]*/,
    ),

    boolean_literal: $ => choice('true', 'false'),

    string_literal: $ => choice(seq(
      '"',
      repeat(choice(token(prec(1, /[^"\\${]+/)), '$', /\\u\{[0-9a-fA-F]+\}/, /\\./, $.interpolation)),
      '"',
    )),

    interpolation: $ => seq(choice('${', '{'), $.expression, '}'),

    expression_statement: $ => seq($.expression, optional(';')),

    statement: $ => choice(
      $.if_statement,
      $.while_statement,
      $.do_statement,
      $.for_statement,
      $.switch_statement,
      $.assign_statement,
      $.destructure_tuple_statement,
      $.destructure_array_statement,
      $.destructure_record_statement,
      $.return_statement,
      $.break_statement,
      $.continue_statement,
      $.fallthrough_statement,
      $.pass_statement,
      $.defer_statement,
      $.unsafe_block,
      $.guard_statement,
      $.throw_statement,
      $.try_statement,
    ),

    if_statement: $ => prec.right(0, seq(
      'if',
      field('condition', $._condition),
      field('consequence', $._block_or_statement),
      optional(seq('else', field('alternative', $._block_or_statement))),
    )),

    while_statement: $ => seq(
      'while',
      field('condition', $._condition),
      field('body', $.block),
    ),

    do_statement: $ => seq(
      'do',
      field('body', $.block),
      'while',
      field('condition', $.expression),
      optional(';'),
    ),

    assign_statement: $ => seq(
      field('left', choice($.identifier, $.member_expression, $.index_expression)),
      field('operator', choice('=', '+=', '-=', '*=', '/=', '%=', '<<=', '>>=', '>>>=')),
      field('right', $.expression),
      optional(';'),
    ),

    for_statement: $ => choice(
      seq(
        'for',
        field('variable', $.identifier),
        'in',
        field('iterable', $._condition),
        field('body', $.block),
      ),
      seq(
        'for',
        '(',
        field('variable', commaSep1($.identifier)),
        ')',
        'in',
        field('iterable', $._condition),
        field('body', $.block),
      ),
      seq(
        'for',
        '(',
        field('variable', $.identifier),
        'in',
        field('iterable', $._condition),
        ')',
        field('body', $.block),
      ),
    ),

    switch_statement: $ => seq(
      'switch',
      field('value', $._condition),
      '{',
      repeat(choice($.case_clause, $.default_clause)),
      '}',
    ),

    case_clause: $ => seq(
      'case',
      field('pattern', $.pattern),
      optional(seq('if', field('guard', $._condition))),
      ':',
      repeat($._declaration_or_statement),
    ),

    default_clause: $ => seq('default', ':', repeat($._declaration_or_statement)),

    pattern: $ => choice(
      $.is_pattern,
      $.expression,
    ),

    is_pattern: $ => prec(1, seq('is', $.type)),

    return_statement: $ => seq('return', optional($.expression), optional(';')),

    break_statement: $ => seq('break', optional(';')),

    continue_statement: $ => seq('continue', optional(';')),

    fallthrough_statement: $ => seq('fallthrough', optional(';')),

    pass_statement: $ => seq('pass', optional(';')),

    defer_statement: $ => seq('defer', field('body', $._block_or_statement)),

    throw_statement: $ => seq('throw', optional(field('value', $.expression)), optional(';')),

    try_statement: $ => seq(
      'try',
      field('body', $.block),
      optional(seq('catch', '(', field('error', $.identifier), ')', field('handler', $.block))),
      optional(seq('finally', field('finalizer', $.block))),
    ),

    identifier: $ => /[a-zA-Z_][a-zA-Z0-9_]*/,
  },
});
