# Getting started with @std/web

No install step: the module ships inside the compiler. Import what you need:

```rnx
import { URL, Headers, statusCode, HttpStatus } from "@std/web";
```

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
import { URL, Headers, statusCode, HttpStatus } from "@std/web";

let u = new URL("https://example.com:8080/a/b?x=1#frag");
print(u.href());
print(u.hostname(), u.port(), u.pathname());
let h = new Headers();
h.set("Content-Type", "text/plain");
print(h.get("content-type").unwrap());
print(statusCode(HttpStatus.NotFound));
```

## Keep going

Read the [overview](overview) for the full picture, then open the API reference for exact signatures.
