# Getting started with @std/wasi

`@std/wasi` is the prelude for `wasm32-unknown-wasi`. Nothing needs to be
imported: `println!` is already in scope once the build targets WASI.

```rnx
fn Main(): Int {
    println!("one line");
    return 0;
}
```

## Build a module

```sh
rnx build --target wasm32-unknown-wasi --release
```

The artifact is a command module with a `_start` export and a `memory`
export. It imports `fd_write` and `proc_exit` from
`wasi_snapshot_preview1` and nothing else.

## Run it

Current Node and Deno releases no longer execute a `.wasm` file as a WASI
program directly: `node --experimental-wasi-unstable-preview1 app.wasm`
treats the file as an ES module import, and `deno run app.wasm` rejects the
`wasi_snapshot_preview1` import. Both hosts expose the preview 1 API
through `node:wasi`, so one small runner serves both. Save it as
`runner.mjs`:

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
node runner.mjs build/release/app.wasm
deno run --allow-read runner.mjs build/release/app.wasm
```

Both print exactly the line the program printed and exit with its status.

## Strings and formatting

Every argument renders the way the host prelude renders it: `Int`, `Bool`,
`Float`, and `String` print their value, and arguments are joined with one
space. The line ends with `\n`.

```rnx
fn Main(): Int {
    let name = "wasi";
    println!("hello", name, 7);
    return 0;
}
```

The output is `hello wasi 7`.

## What is missing

Container rendering falls back to a placeholder, so `Array`, `Map`, and
class instances are not pretty-printed yet. `@std/fs`, `@std/process`,
`@std/net`, and `@std/sync` are not part of the target prelude: their
capabilities are not implemented on this tier 3 target.
