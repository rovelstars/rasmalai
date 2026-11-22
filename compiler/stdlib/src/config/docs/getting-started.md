---
title: Getting started with @std/config
description: Read a .config document, navigate it, change it, and write it back.
section: Getting started
order: 1
---

# Getting started with @std/config

A `.config` document starts with `export default` and an object. Constants
may come before or between declarations, and their values are inlined when
the document is evaluated.

```rnx
import { Config } from "@std/config"

let text = "const version = \"0.4.0\"\nexport default \{ project: \{ name: \"colony\", version: version } }"
let doc = Config.parse(text)
let project = doc.object("project")
println!(project?.text("name"), project?.text("version"))
```

That prints `colony 0.4.0`.

## Read fields

`ConfigObject` and `ConfigList` carry typed accessors that return `null`
for a missing field or a type mismatch. Combine them with `?.` so an
incomplete document does not abort the program:

```rnx
import { Config } from "@std/config"

let doc = Config.parse("export default \{ server: \{ host: \"127.0.0.1\", port: 9000 } }")
let server = doc.object("server")
println!(server?.text("host") ?? "localhost")
println!(server?.number("port") ?? 8080)
```

## Change and write

Parsed objects and lists are mutable. Change a value, add a field, then
render the canonical document:

```rnx
import { Config, ConfigList } from "@std/config"

let doc = Config.parse("export default \{ name: \"demo\" }")
doc.set("version", "0.2.0")
let list = new ConfigList()
list.push("linux")
doc.set("targets", list)
println!(Config.stringify(doc))
```

The writer emits bare keys where the grammar allows them, quotes the rest,
and escapes strings so the result parses back to the same values.

## Files stay outside

The module never opens a file. Read the text with `@std/fs`, pass it to
`Config.parse`, and write the rendered document back:

```rnx
import { Config } from "@std/config"

let text = "export default \{ name: \"demo\" }"
let doc = Config.parse(text)
let rendered = Config.stringify(doc)
println!(rendered.contains("export default"))
```

That separation keeps the parser usable in tests, on targets without a
filesystem, and in build steps that generate configuration from another
source.
