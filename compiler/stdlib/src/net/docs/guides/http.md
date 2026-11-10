---
title: "HTTP message shaping"
description: Build HTTP/1.1 request lines and read status lines with the @std/net/http submodule.
section: "HTTP"
order: 40
---

# HTTP message shaping

An HTTP exchange starts with a line of text. On the request side it is a
method, a target, and a version. On the response side it is a version, a
status, and a reason phrase. `@std/net/http` builds and reads those lines
without opening a socket.

## Request and status lines

`encodeRequestLine()` joins its two arguments to the HTTP/1.1 version and
terminates the result with CRLF. `statusCode()` reads the three-digit code
from a response status line.

```rnx
import { encodeRequestLine, statusCode, isSuccess } from "@std/net/http";

println!(encodeRequestLine("GET", "/health"));
println!(statusCode("HTTP/1.1 200 OK"));
println!(statusCode("not a status line"));
println!(isSuccess(200), isSuccess(404));
```

```text
GET /health HTTP/1.1

200
-1
true false
```

`statusCode()` returns `-1` for anything that is not a version, a space, and
three ASCII digits. It does not throw, so callers can branch on the sentinel.
`isSuccess()` tests 200 to 299 and returns false for `-1`.

## Prepare a request

`HttpTarget` holds the three parts a client needs: the host to dial, the port,
and the origin-form path to put on the request line.

```rnx
import { HttpTarget, encodeRequestLine } from "@std/net/http";

let target = HttpTarget("example.com", 80, "/health");
let req = encodeRequestLine("GET", target.path)
    + "Host: " + target.host + "\u{D}\u{A}"
    + "Connection: close" + "\u{D}\u{A}"
    + "\u{D}\u{A}";

println!(req);
```

```text
GET /health HTTP/1.1
Host: example.com
Connection: close

```

The final blank line ends the header block. `TcpStream.write()` wants bytes,
not text, so encode the string before sending it. `@std/web` exports
`strBytes()` for exactly that, and `bytesToString()` for the reply. Dial with
`TcpStream.connect(target.host, target.port)`, write the encoded request,
then read until the response framing says the body is complete.

This submodule only shapes text. It does not parse header fields, follow
redirects, or decide body length. For a complete client over TCP and TLS use
`@std/web`'s `fetch()`, which builds the request, parses the status line and
headers, and handles `Content-Length`, close-delimited, and chunked bodies.
