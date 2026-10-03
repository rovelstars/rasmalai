# Frontend

Lexing, parsing, semantic analysis, and project management for `.rnx`
source. Output is a checked AST ready for `lir` lowering.

## Files

- `lexer.rs`, `token.rs` — tokenizer.
- `parser.rs` — recursive-descent parser producing `ast.rs`.
- `ast.rs` — the abstract syntax tree all later stages consume.
- `semantic.rs`, `check.rs` — name resolution and type checking; errors
  carry `diagnostics::Code`s.
- `desugar.rs`, `async_expand.rs` — normalize surface syntax (`async`/`await`,
  closures, patterns) before lowering.
- `modules.rs` — the module graph: file discovery, import resolution,
  cycle handling.
- `prelude.rs` — implicit imports every module gets.
- `project/` — `Project.config` manifests, workspace members, dependency
  resolution (`deplock.rs`), registry fetch (`fetch.rs`), vendoring.
- `capabilities.rs`, `security.rs` — permission model (`sys.exec`,
  `env.read`, …) and path sandboxing.
- `lsp_*.rs`, `semantic_tokens.rs`, `highlight.rs` — editor support:
  completion, go-to-definition, symbols, highlighting.
- `fmt.rs`, `lint.rs` — formatter and linter.
- `doc.rs`, `pack.rs`, `tar.rs`, `gzip.rs` — documentation rendering and
  package archives.
- `sarif.rs`, `profiler.rs`, `bench.rs`, `checksum.rs`, `header.rs`,
  `harness.rs` — CI output formats, profiling hooks, benchmarking,
  integrity checks.

## Tests

`tests/` holds parser, semantic, module, workspace, and project suites.
Corpus tests that used `examples/` were removed with that directory;
coverage for the same grammar lives in inline-source tests such as
`parses_switch_try_defer_guard`.
