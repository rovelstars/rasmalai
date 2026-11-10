---
title: "URLs"
description: Parse absolute URLs, resolve relative references, and edit the parts of a URL.
section: "URLs"
order: 20
---

# URLs

A URL arrives as text and is used in pieces: the host to dial, the path to
request, the query to read. Cutting that text by hand breaks on ports,
fragments, and dot segments. `URL` parses it once and gives each part a
typed accessor.

## Parse a URL

```rnx
import { URL } from "@std/web";

let u = new URL("https://example.com:8080/a/b?x=1#frag");
println!(u.protocol(), u.host(), u.pathname());
println!(u.search(), u.hash());
println!(u.origin());
```

```text
https: example.com:8080 /a/b
?x=1 #frag
https://example.com:8080
```

The fields keep the URL as written. `port()` returns `8080`, and an empty
string when the text had no port. `host()` adds a port only when it differs
from the scheme default, so `https://example.com:443/x` serializes as
`https://example.com/x`. `origin()` returns the string `"null"` for a
relative URL or a URL with no host, not a null value.

## Resolve a relative reference

Pass a base as the second argument. Resolution follows RFC 3986: the
reference can replace the path, borrow the scheme and authority, or keep the
base path and change only the query or fragment.

```rnx
import { URL } from "@std/web";

let base = new URL("https://example.com/docs/manual/index.html");
println!(new URL("../guide", base.href()).href());
println!(new URL("//cdn.example.com/lib.js", base.href()).href());
println!(new URL("?page=2", base.href()).href());
println!(new URL("#top", base.href()).href());
```

```text
https://example.com/docs/guide
https://cdn.example.com/lib.js
https://example.com/docs/manual/index.html?page=2
https://example.com/docs/manual/index.html#top
```

Resolution removes `.` and `..` segments. Parsing an absolute URL does not,
so `new URL("https://a.com/a/../b").pathname()` stays `/a/../b`.

## Change one part

`setPathname()`, `setSearch()`, and `setHash()` update a parsed URL in place.
The setters accept the value with or without its leading separator.

```rnx
import { URL } from "@std/web";

let u = new URL("https://example.com/old?x=1");
u.setPathname("new");
u.setSearch("?page=2");
u.setHash("#top");
println!(u.href());
```

```text
https://example.com/new?page=2#top
```

A path set on a hosted URL gets a leading slash if it lacks one. An empty
path becomes `/` on a hosted URL. Dot segments are not removed by the
setters.

## What URL does not do

Percent escapes stay exactly as written. Use `decodeComponent()` when you
need the decoded text. Hosts fold ASCII case only; there is no IDNA, no IPv6
literal parsing, and no credentials support. `WebSocket` URLs parse as
ordinary URLs but the package does not implement a WebSocket client.

The lower-level parser is available as `parseAbsolute()` plus `UrlParts` when
you want the parts without constructing a `URL`. `defaultPort()` gives the
default for a scheme, and `removeDotSegments()` normalizes a path on its own.
