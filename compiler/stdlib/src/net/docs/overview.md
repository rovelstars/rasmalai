---
title: "@std/net"
description: TCP sockets, TLS sessions, DNS lookups, and HTTP message shaping.
section: Overview
order: 0
---

# @std/net

`@std/net` is the socket layer. `TcpStream` reads and writes bytes on a
connected TCP socket, `TcpListener` waits for incoming connections,
`TcpStream.connect()` opens an outgoing one, `TlsStream` upgrades a connected
socket to TLS, and `Dns.lookup()` resolves a name. A submodule,
`@std/net/http`, shapes request lines and status lines without touching a
socket.

```rnx
import { TcpListener } from "@std/net";

let listener = TcpListener.bind("127.0.0.1", 0);
println!(listener.port() > 0);
listener.close();
```

Bytes are `Int` values from 0 to 255, one per octet. The socket types do not
decode text. Encode a request line or a JSON body before `write()`, and
decode the reply after `read()`. Framing belongs to the protocol you speak:
TCP delivers a byte stream, not messages.

The sockets are non-blocking, but the calls are not. A call that has to wait
parks the calling thread; one reactor thread keeps the sockets in the kernel
poll set and hands back a result. Nothing spins, and other threads keep
running, but the thread that called `read()` executes nothing else until that
read finishes. There is no timeout and no way to cancel a parked call. Close
the stream from another thread to unblock it.

Failures reject with a `String` message such as `connect failed: ...`.
`TcpListener.bind()` is the exception: it never waits, so it throws instead.
There is no exception type to match on.

The package does not exist on WebAssembly. There is no reactor there, so DNS
lookups reject, `bind()` throws, and the other calls report that networking is
not supported.

## What is inside

- `TcpStream`: `connect()`, `read()`, `write()`, `handleOf()`, `close()`.
- `TcpListener`: `bind()`, `accept()`, `port()`, `close()`.
- `TlsStream`: `connect()`, `read()`, `write()`, `handleOf()`, `close()`.
- `Dns`: `lookup()`.
- `@std/net/http`: `HttpTarget`, `encodeRequestLine()`, `statusCode()`,
  `isSuccess()`.

## Guides

- [TCP sockets](guides/tcp) covers binding, accepting, reading, writing, and
  framing.
- [TLS sessions](guides/tls) covers certificate verification and the upgrade
  from a plain stream.
- [HTTP message shaping](guides/http) covers the `@std/net/http` helpers and
  how they fit with a socket.
