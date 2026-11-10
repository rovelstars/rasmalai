---
title: "Requests and responses"
description: Build a Response by hand, describe a request with RequestOptions, and send it with fetch.
section: "HTTP"
order: 50
---

# Requests and responses

A response is more than a body. Code that reads one needs the status, the
reason phrase, the headers, and a body that is already complete. `Response`
holds all four. `RequestOptions` holds the parts of a request that vary:
method, extra headers, and body. `fetch()` ties them to a socket.

## Build a response without a server

Constructing a `Response` is the shortest way to test code that consumes one.
Nothing is validated, so any status number and an empty body are allowed.

```rnx
import { Response, Headers, isSuccess } from "@std/web";

let r = new Response(
    404,
    "Not Found",
    Headers.fromPairs([["Content-Type", "text/plain"]]),
    "no such page"
);

println!(r.status, r.statusText, isSuccess(r.status));
println!(r.headers.get("content-type") ?? "");
println!(r.text());
```

```text
404 Not Found false
text/plain
no such page
```

`text()` returns the stored body. `bytes()` returns the original socket bytes
when the response came from `fetch()`, and otherwise encodes `body` as UTF-8.
`json()` parses the body with `@std/json`; check the status and body first if
the payload might not be JSON, because a parse failure aborts rather than
returning an error value.

## Send a request

`fetch()` accepts an absolute `http://` or `https://` URL and an optional
`RequestOptions`. Passing null or omitting the options sends a GET with no
extra headers and no body.

```rnx
import { fetch, RequestOptions, isSuccess } from "@std/web";

let opts = new RequestOptions("POST");
opts.headers.set("Content-Type", "application/json");
opts.body = "\{\"name\":\"ada\"}";

let r = await fetch("https://example.com/api/people", opts);
if isSuccess(r.status) {
    println!(r.text());
} else {
    println!("request failed:", r.status, r.statusText);
}
```

`fetch()` writes `Host`, `Connection: close`, `User-Agent: Rasmalai/1.0`,
and `Accept: */*`, then the headers from the options. It adds
`Content-Length` when the body is not empty and sends the body after a blank
line. The method string is sent as written, so use uppercase.

A 4xx or 5xx status resolves normally. A request that cannot be formed
rejects before it connects: an unsupported scheme, a URL with no host, or a
bad port. Transport and framing failures reject too: DNS or connect failure,
a short write, a closed connection before the headers, a malformed status
line or content length, an unsupported transfer encoding, and a truncated
body. There is no timeout, so a server that accepts and never answers leaves
the call waiting.

The client supports HTTP/1.1 over TCP and TLS. It disables keep-alive on every
request, follows no redirects, sends no cookies, does not decompress bodies,
and has no HTTP/2 support. A redirect comes back as a 3xx response for you to
handle. Every call opens a fresh connection.

## Lower-level pieces

`fetchInner()`, `fetchOverTcp()`, and `fetchOverTls()` are exported for code
that already has a prepared request string. The byte helpers `strBytes()`,
`sliceBytes()`, `bytesToString()`, `readMore()`, `readHeaders()`, and
`readBodyInto()` are the same functions `fetch()` uses. Most programs should
call `fetch()` and skip them.
