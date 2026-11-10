# @std/wasi

Prelude and console output for `wasm32-unknown-wasi`.

A WASI command module starts with no process around it. There is no stdout
file descriptor to inherit, no `@std/io`, and no `@std/fs` unless the host
grants those directories. Everything the module shows a user leaves through
`wasi_snapshot_preview1.fd_write`, and the module ends by calling
`proc_exit`. `@std/wasi` is the small surface for that: two call macros and
four plain functions.

```rnx
fn Main(): Int {
    println!("hello wasi");
    return 0;
}
```

Build it:

```sh
rnx build --target wasm32-unknown-wasi --release
```

Current Node and Deno releases no longer run a `.wasm` file as a WASI
program directly, so the module goes through the preview 1 API. Save this
as `runner.mjs`:

```js
import { WASI } from "node:wasi";
import { readFile } from "node:fs/promises";
const wasi = new WASI({ version: "preview1", args: [], env: {} });
const bytes = await readFile(process.argv[2]);
const module = await WebAssembly.compile(bytes);
const instance = await WebAssembly.instantiate(module, wasi.getImportObject());
process.exit(wasi.start(instance));
```

```sh
node runner.mjs build/release/hello.wasm
deno run --allow-read runner.mjs build/release/hello.wasm
```

## What is inside

- `println!`, the call macro that renders its arguments as one line
- `wasi_println!`, the same macro under a distinct name
- `eprintln!`, the same rendering on descriptor 2
- `printLine(text)`, `printText(text)`, `printErrorLine(text)`,
  `printErrorText(text)` for a `String` you already hold

The foundation names (`Int`, `String`, `Array`, `Map`, and the rest) stay
ambient because this package is the target prelude, and `Date` comes from
`@std/time`.

## When to use

Use it in a program built with `rnx build --target wasm32-unknown-wasi`.
The target is tier 3: the in-house linker builds modules that print and
exit, and the wider `@std` surface is not ported yet. Importing a package
that needs a capability the descriptor does not declare fails at the
import, before the link runs.
