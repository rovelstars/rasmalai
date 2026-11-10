---
title: "Query strings"
description: Read, edit, and encode URL query parameters with URLSearchParams.
section: "URLs"
order: 30
---

# Query strings

Query strings repeat keys, mix encoded characters, and change while a
program builds a request. `URLSearchParams` keeps the pairs in order, lets a
name repeat, and re-encodes the result with form rules.

## Read parameters

```rnx
import { URLSearchParams } from "@std/web";

let p = new URLSearchParams("?tag=a&tag=b&q=x+y");
println!(p.size(), p.getAll("tag").join(","));
println!(p.get("q") ?? "");

p.set("page", "2");
p.append("tag", "c");
println!(p.toString());
```

```text
3 a,b
x y
tag=a&tag=b&q=x+y&page=2&tag=c
```

`get()` returns the first value for a name, or null. `getAll()` returns every
value, which is how a repeated key stays visible. `set()` drops the earlier
entries for that name and appends one value; `append()` keeps them. Names and
values are stored decoded, so `x+y` reads back as `x y`, and `toString()`
encodes the space as `+` again. The output has no leading `?`; add that
yourself when writing a URL.

`has()` tests membership, `delete()` removes every entry for a name,
`size()` counts entries, and `keys()`, `values()`, and `entries()` return
copies in insertion order. `sort()` orders by name and keeps duplicate names
in the order they were added.

## Build a query from a map

`URLSearchParams.fromMap()` copies a `Map` in its own order. Values are taken
as decoded text, so a value containing `%` is not decoded a second time.

```rnx
import { URL, URLSearchParams } from "@std/web";
import { Map } from "@std/collections";

let m = new Map<String, String>();
m.set("q", "rasmalai");
m.set("page", "2");

let p = URLSearchParams.fromMap(m);
let u = new URL("https://example.com/search");
u.setSearch(p.toString());
println!(u.href());
```

```text
https://example.com/search?q=rasmalai&page=2
```

`URL.searchParams()` returns a snapshot of a URL's query, not a live view.
Change the snapshot, then write it back with `setSearch()`.

## Encode and decode text

`encodeComponent()` and `decodeComponent()` are the primitives underneath the
parameter table. Pass `form: true` for query-string rules, where a space
becomes `+`; pass `false` for URL rules, where it becomes `%20`.

```rnx
import { encodeComponent, decodeComponent } from "@std/web";

println!(encodeComponent("a b&c=d", false));
println!(encodeComponent("a b&c=d", true));
println!(decodeComponent("%E2%9C%93", false).length());
println!(decodeComponent("a+b", true));
println!(decodeComponent("a%2Gb", false));
```

```text
a%20b%26c%3Dd
a+b%26c%3Dd
1
a b
a%2Gb
```

Decoding never throws. A malformed escape is copied through as a literal
percent sign and the rest of the text is decoded normally.
