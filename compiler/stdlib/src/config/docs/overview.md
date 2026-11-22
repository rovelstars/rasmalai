---
title: The @std/config package
description: Parse and write the sandboxed .config document format without touching the filesystem.
section: Overview
order: 0
---

# The @std/config package

`@std/config` reads and writes the same document format the toolchain uses
for `Project.config` and `Project.deplock`. That format is the sandboxed
subset of rnx: `const` bindings plus one `export default` object, over
strings, integers, booleans, objects, and arrays. It is data, not code.
Both files use one shape, so the parser handles either.

The package is pure. It takes text and returns values, or takes values and
returns text. Reading the file belongs to `@std/fs`, and writing it back is
the same call in reverse. Keep those two concerns apart and the parser
works anywhere: a test with no files, a sandboxed target, or a build step
that assembles configuration before a filesystem exists.

```rnx
import { Config } from "@std/config"

let text = "export default \{ app: \{ name: \"demo\", port: 8080 } }"
let doc = Config.parse(text)
println!(doc.object("app")?.text("name"), doc.object("app")?.number("port"))
```

That prints `demo 8080`.

## The value model

Objects decode to `ConfigObject`, arrays to `ConfigList`, and scalars stay
`String`, `Int`, and `Bool`. Both containers keep document order, so
`keys()` and iteration follow the order the file wrote, not a hash order.

Read a field with the typed accessors instead of casting:

```rnx
import { Config } from "@std/config"

let doc = Config.parse("export default \{ ports: [8080, 8081], debug: true }")
let ports = doc.list("ports")
println!(ports?.number(0), doc.flag("debug"))
```

`text`, `number`, `flag`, `object`, and `list` return `null` when the field
is missing or holds a different type. Plain `get` returns the raw value.
`set`, `remove`, `push`, and `len` change the parsed value in place.

## target

Manifests may branch on the build target. `Config.parse` evaluates
`target.os`, `target.arch`, and `target.env` against the host.
`Config.parseWith` takes the three strings explicitly, which is what
reproducible tooling should use:

```rnx
import { Config } from "@std/config"

let text = "const lib = target.os == \"windows\" ? \"zlibstatic\" : \"z\"\nexport default \{ native: lib }"
let doc = Config.parseWith(text, "windows", "x86_64", "msvc")
println!(doc.text("native"))
```

That prints `zlibstatic`.

## Writing

`Config.stringify` renders a canonical document. Constants, spreads,
ternaries, and `switch` are resolved, keys keep their order, and values use
four-space indentation:

```rnx
import { Config } from "@std/config"

let doc = Config.parse("export default \{ name: \"demo\", tags: [\"a\", \"b\"] }")
println!(Config.stringify(doc))
```

The output parses back to an equal document, so a read-modify-write cycle
does not drift.

## Errors

`Config.parse` stops the program with the parse message when the document
is invalid. `Config.tryParse` returns a `Result<ConfigObject, String>`
instead, with the line and column in the error. Both reject every construct
outside the subset: function calls, closures, imports, `let`, loops, float
literals, string interpolation, and indexing.
