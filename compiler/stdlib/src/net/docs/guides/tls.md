---
title: "TLS sessions"
description: Upgrade a TCP stream to TLS, verify the server certificate, and exchange encrypted bytes.
section: "TLS"
order: 30
---

# TLS sessions

A plain TCP socket carries bytes anyone on the path can read. A TLS session
adds a handshake, certificate verification, and encryption. `TlsStream` owns
that state over an already connected `TcpStream`.

## Upgrade a connected socket

`TlsStream.connect()` takes over the TCP socket. The handle moves into the
session, so the original stream must not be used again, not even to close it.
The domain argument is the name the certificate has to match.

```rnx
import { TcpStream, TlsStream } from "@std/net";

let raw: TcpStream = await TcpStream.connect("example.com", 443);
let tls: TlsStream = await TlsStream.connect(raw, "example.com");
println!(tls.handleOf() >= 0);
tls.close();
```

```text
true
```

A DNS name is sent as SNI as well as checked against the certificate. An IP
literal is not sent as SNI, and a server presenting a certificate for an IP
address needs a matching IP subject alternative name.

The handshake runs in the reactor and parks the calling thread until it
finishes. Read and write then behave like they do on a plain stream: one
reader and one writer, the same 1 to 65536 byte read cap, and the same
byte-at-a-time write path. `read()` returns plaintext with record boundaries
and padding removed.

## Test against a local certificate

Certificates are verified against the Mozilla root set that ships with the
runtime. There is no way to skip verification or add a private authority from
Rasmalai code. For a local server with a self-signed certificate, point
`RNX_TEST_TLS_CA_DER` at a DER-encoded CA file. That replaces the root set
with the single certificate.

```rnx
import { TcpStream, TlsStream } from "@std/net";

let raw: TcpStream = await TcpStream.connect("localhost", 8443);
let tls: TlsStream = await TlsStream.connect(raw, "localhost");
let reply: Array<Int> = await tls.read(4096);
println!(reply.length() > 0);
tls.close();
```

Run it with the variable set before the process starts, because the root
store is built once per process and the first TLS call fixes it:

```sh
RNX_TEST_TLS_CA_DER=/path/to/ca.der rnx run main.rnx
```

Client certificates are not supported. A handshake that cannot verify the
server rejects before the request is written.

## Close behaviour

`close()` queues a `close_notify` alert, closes the socket, and releases the
handle. The socket is dropped without a final flush, so the peer may never
receive the alert. Treat a clean shutdown as best effort. Parked operations
on the session reject, later ones reject as well, and closing twice does
nothing.
