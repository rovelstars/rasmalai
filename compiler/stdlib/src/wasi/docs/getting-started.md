---
title: "Getting started with @std/wasi"
description: Build and run a first WASI command module with rnx and a preview 1 runner.
section: Getting started
order: 10
---

# Getting started with @std/wasi

`@std/wasi` is the prelude for `wasm32-unknown-wasi`, so there is no import
line to write. The build loads the package as the target prelude, and
`println!` is already in scope. An explicit
`import { println } from "@std/wasi";` is not accepted, because the package is
not an ordinary library import on this target.

## Copy this

```rnx
fn Main(): Int {
    println!("one line");
    return 0;
}
```

Install the target pack if `rnx target list` reports it missing, then build:

```sh
rnx sysroot add wasm32-unknown-wasi
rnx build --target wasm32-unknown-wasi --release
```

The artifact is a command module with a `_start` export and a `memory`
export. It imports `fd_write` and `proc_exit` from
`wasi_snapshot_preview1` and nothing else.

## Run it

Current Node and Deno releases no longer execute a `.wasm` file as a WASI
program directly. Both expose the preview 1 API through `node:wasi`, so one
small runner serves both. Save it as `runner.mjs`:

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

Both print:

```text
one line
```

They exit with the value `Main` returned.

## Keep going

Read [Console streams](guides/streams) next for standard error, the print
functions for strings, and the formatting rules.
