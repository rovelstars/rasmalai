# @std/web

URL, query string, header, status, and HTTP request primitives.

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

Read the [overview](/docs/@std/web/overview) or open the [API reference](/docs/@std/web/api).
