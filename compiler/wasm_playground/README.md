# WASM playground

`cdylib` build of the compiler frontend and interpreter for the browser
playground on the website.

## API (`src/lib.rs`)

- `tokenize(source)` — source text to a token stream (JSON).
- `check(source)` — lex, parse, and semantically check; returns
  diagnostics (JSON).
- `run(source)` — execute in the interpreter; returns output (JSON).

Threading, sockets, and the OS interface are unavailable in the browser:
the crate builds against `reactor_wasm.rs` stubs, and platform-dependent
behavior is documented as such rather than emulated. `tests/api_examples.rs`
validates responses against the website's stdlib API data.
