---
title: "Getting started with @std/json"
description: A first program that parses JSON, reads an object field, and writes canonical text.
section: Getting started
order: 10
---

# Getting started with @std/json

No install step: the module ships inside the compiler. Import the codec:

```rnx
import { JSON } from "@std/json";
```

## Copy this

Save it as `main.rnx` in a project and run `rnx run main.rnx`:

```rnx
import { JSON } from "@std/json";

println!(JSON.stringify(JSON.parse("12345")));

let obj = JSON.parseObject("\{\"count\": 41}");
println!(obj.get("count"));
println!(obj.len(), obj.keys().join(","));
```

The program prints:

```text
12345
41
1 count
```

`JSON.parse()` accepts any top level value. An object becomes a handle, and
`parseObject()` turns that handle into a `Map<String, Any>` so `get()` can
read a field. The map keeps keys in byte-sorted order, which is why `count`
is the only key here.

The `\{` and `\}` escapes keep JSON braces from being read as string
interpolation. JSON text inside a Rasmalai string needs those escapes.

## Keep going

Read [Parsing](guides/parsing) next. It covers arrays, numbers, typed decoding
into a struct, and the errors a malformed document produces.
