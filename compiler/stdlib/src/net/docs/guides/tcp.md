---
title: "TCP sockets"
description: Connect, accept, read, write, and frame data on TCP streams.
section: "Sockets"
order: 20
---

# TCP sockets

TCP gives you a stream of bytes, not a sequence of messages. A single
`read()` can return half a request, three messages, or an empty array after
the peer closes. Code that assumes one read equals one message works in a
test and fails on a network. `TcpStream` exposes the stream as it is, so the
protocol above it owns the framing.

## Open both ends

```rnx
import { TcpListener, TcpStream } from "@std/net";

let listener = TcpListener.bind("127.0.0.1", 0);
let client: TcpStream = await TcpStream.connect("127.0.0.1", listener.port());
let server: TcpStream = await listener.accept();

println!(await client.write([104, 105]));
println!(await server.read(2));

client.close();
server.close();
listener.close();
```

```text
2
[104, 105]
```

`bind()` returns a listener directly because binding never waits. `connect()`
and `accept()` return promises that are already settled when the call
returns, so `await` resolves or rejects immediately; the wait happened inside
the call.

## Read until the peer closes

A reader has to loop. `read()` parks until at least one byte arrives and
returns whatever the socket had. The cap is clamped into 1 to 65536 bytes.
An empty array means the peer closed its end, and later reads keep returning
empty arrays until you close the stream.

```rnx
import { TcpListener, TcpStream } from "@std/net";

let listener = TcpListener.bind("127.0.0.1", 0);
let client: TcpStream = await TcpStream.connect("127.0.0.1", listener.port());
let server: TcpStream = await listener.accept();

await client.write([1, 2, 3]);
client.close();

let total = 0;
let chunk: Array<Int> = await server.read(64);
while chunk.length() > 0 {
    total = total + chunk.length();
    chunk = await server.read(64);
}
println!(total);

server.close();
listener.close();
```

```text
3
```

This loop works when the peer closes after sending. A protocol that keeps the
connection open needs its own boundary: a length prefix, a delimiter, or a
fixed-size header followed by a body. Parse that boundary out of the byte
array yourself, because the socket does not preserve write boundaries.

## Rules that affect real programs

`write()` sends one byte per reactor round trip. Backpressure is automatic
and a full send buffer parks the caller, but bulk payloads are slow: 4 KiB
costs 4096 wakeups. Each byte is truncated to its low 8 bits, so 300 travels
as 44.

There is one reader and one writer per stream. The reactor keeps a single
read buffer per socket, so two readers race for the same bytes and two
writers interleave theirs. Move a stream to one thread rather than sharing
it.

There is no timeout anywhere in this package. A peer that connects and then
goes quiet parks the calling thread for as long as it holds the socket open.
Closing the stream from another thread makes the parked call reject. A
listener works the same way: a parked `accept()` waits until a peer arrives,
and closing the listener is the way to stop it.

`bind()` follows the platform defaults. On Unix it sets address reuse, so a
port a previous socket left behind can be rebound; a port another process is
listening on still fails. The accept backlog is fixed.

## Addresses and DNS

`TcpStream.connect()` and `TcpListener.bind()` resolve their host before the
socket starts. An IP literal is used as written. A hostname goes through the
system resolver on the calling thread and blocks until it returns; when a
name has both IPv4 and IPv6 results, the IPv4 address wins. Use
`Dns.lookup()` to run the lookup on a worker thread instead.

```rnx
import { Dns } from "@std/net";

println!(await Dns.lookup("127.0.0.1"));
println!(await Dns.lookup("::1"));
println!(await Dns.lookup("localhost"));
```

`Dns.lookup()` returns one address as text. It has no cache, no TTL, no way
to ask for a particular address family, and no reverse, SRV, MX, or TXT
lookup. A literal comes back in normalized form, so
`"0:0:0:0:0:0:0:1"` resolves to `"::1"`.
