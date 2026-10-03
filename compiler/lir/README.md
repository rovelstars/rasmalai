# LIR

Low-level intermediate representation: the typed contract between the
frontend and the three backends (interpreter, Cranelift JIT, LLVM AOT).

## Files

- `instr.rs` — instruction set, locals, types, terminators.
- `lower/` — AST-to-LIR lowering, split by responsibility:
  - `mod.rs` — entry point (`lower()`), context, shared type helpers.
  - `stmt.rs` — statements, loops, `switch`, function bodies.
  - `expr.rs` — expressions, operators, `++`/`--`, compound assignment.
  - `calls.rs` — calls, closures, generics, foreign `from native` linking.
  - `views.rs` — array-view checks, boxing/unboxing, retain/release
    emission, interface checks.
- `opt.rs` — optimizer driver plus seven passes: `arc_opt.rs` (reference
  counting), `bce.rs`, `escape.rs`, `licm.rs`, `sroa.rs`, `tco.rs`,
  `temp_sweep.rs`.
- `verify.rs` — structural validation of lowered modules; backends assume
  verified input.

Method visibility across `lower/` submodules is `pub(super)`: the split is
organizational, the API surface is still `lir::lower::lower`.
