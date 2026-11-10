---
title: "Getting started with @std/web"
description: A first program that parses a URL, stores a header, and reads a status code.
section: Getting started
order: 10
---

# Getting started with @std/web

No install step: the module ships inside the compiler. Import the names you
need:

```rnx
import {
    URL,
    Headers,
    HttpStatus,
    statusCode,
    statusReason
} from "@std/web";
```

## Copy this

Save it as `main.rnx` in a project and run `rnx run main.rnx`:

```rnx
import {
    URL,
    Headers,
    HttpStatus,
    statusCode,
    statusReason
} from "@std/web";

let u = new URL("https://example.com:8080/a/b?x=1#frag");
println!(u.href());
println!(u.hostname(), u.port(), u.pathname());

let h = new Headers();
h.set("Content-Type", "text/plain");
println!(h.get("content-type") ?? "missing");

println!(statusCode(HttpStatus.NotFound), statusReason(HttpStatus.NotFound));
```

The program prints:

```text
https://example.com:8080/a/b?x=1#frag
example.com 8080 /a/b
text/plain
404 Not Found
```

The URL keeps the port because 8080 is not the default for `https`. The
header lookup succeeds with a lowercase name because `Headers` folds names
before storing them.

## Keep going

Read the [URLs guide](guides/urls) next. It shows relative resolution and the
accessors that rebuild a URL after you change one part.
