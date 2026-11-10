---
title: "@std/web"
description: URL, query string, header, status, and HTTP request primitives.
section: Overview
order: 0
---

# @std/web

`@std/web` covers the text side of HTTP. Parse a URL, resolve a relative
reference, edit a query string, store case-insensitive headers, map status
codes, or send a request with `fetch()`. The URL, header, status, and query
helpers are pure string and table work, so they give the same answer on every
backend and never open a socket. The request helpers do use a socket through
`@std/net`.

```rnx
import { URL } from "@std/web";

let u = new URL("../guide?page=2", "https://example.com/docs/manual/index.html");
println!(u.href());
println!(u.hostname(), u.pathname());
```

`URL` keeps the parts apart and serializes them again. `URLSearchParams`
owns query parameters as ordered name/value pairs. `Headers` folds names to
lowercase so `Content-Type` and `content-type` are one entry. `HttpStatus`
names the common status codes, and `statusCode()`, `statusReason()`, and
`statusFromCode()` move between a variant and its wire form.

`Response` holds a status, headers, and a body that is already read. Build one
yourself to test code that consumes responses, or receive one from `fetch()`.
`RequestOptions` carries the method, extra headers, and body for a request.

The package declares the `net` capability because `fetch()` connects through
`@std/net`. The URL, header, status, and query helpers themselves do no I/O.
`WebSocket` is not part of the package yet.

## What is inside

The main types are `URL`, `URLSearchParams`, `Headers`, `Response`, and
`RequestOptions`. `HttpStatus` is the status enum. `UrlParts` is the holder
the parser fills in; use `URL` instead of building one directly.

The free functions cover status codes (`statusCode()`, `statusReason()`,
`statusFromCode()`, `isSuccess()`, `isClientError()`, `isServerError()`),
percent encoding (`encodeComponent()`, `decodeComponent()`), ASCII folding
(`lowerAscii()`), URL resolution (`parseAbsolute()`, `parseAuthority()`,
`removeDotSegments()`, `mergePaths()`, `defaultPort()`), and the byte helpers
`fetch()` uses to read a socket.

## Guides

- [URLs](guides/urls) covers parsing, resolving, and editing a `URL`.
- [Query strings](guides/query-strings) covers `URLSearchParams` and percent
  encoding.
- [Headers and status codes](guides/headers) covers `Headers`, `HttpStatus`,
  and the status helpers.
- [Requests and responses](guides/requests) covers `RequestOptions`,
  `Response`, and `fetch()`.
