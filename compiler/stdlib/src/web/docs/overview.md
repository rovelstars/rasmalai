# @std/web

HTTP and URL primitives: status codes, header tables, URL parsing,
query strings, and a small HTTP/1.1 client.

Every item except `fetch()` is pure string and table work: no I/O, no
capabilities, same answer on every backend. Use them to build and
inspect requests offline. `fetch()` opens a socket through `@std/net`,
so a project that calls it must grant the `net` capability domain.
`WebSocket` is out of scope until the network RFC lands (see
requirements 12_STDLIB).

```rnx
import { URL } from "@std/web";

let u = new URL("https://example.com/a?x=1");
print(u.hostname(), u.pathname());
```

## What is inside

- `HttpStatus`
- `Headers`
- `UrlParts`
- `URL`
- `URLSearchParams`
- `Response`
- `RequestOptions`
- free functions: `statusCode()`, `statusReason()`, `statusFromCode()`, `isSuccess()`, `isClientError()`, `isServerError()`, `lowerAscii()`, `isDigitCode()`

## When to use

Reach for @std/web for pure URL and header work with zero I/O: parsing hrefs, resolving relative references, manipulating query strings, or mapping status codes. fetch() and WebSocket stay out until the network RFC lands.

It covers: HttpStatus codes; Case-insensitive Headers; URL parse/resolve; URLSearchParams.
