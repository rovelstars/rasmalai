---
title: "Getting started with @std/net"
description: A first program that binds a listener and exchanges bytes over TCP.
section: Getting started
order: 10
---

# Getting started with @std/net

No install step: the module ships inside the compiler. Import the socket
types you need:

```rnx
import { TcpListener, TcpStream } from "@std/net";
```

## Copy this

The program opens a listener on an ephemeral port, connects a client to it,
accepts the connection on the server side, and sends two bytes:

```rnx
import { TcpListener, TcpStream } from "@std/net";

let listener = TcpListener.bind("127.0.0.1", 0);
println!(listener.port() > 0);

let client: TcpStream = await TcpStream.connect("127.0.0.1", listener.port());
let server: TcpStream = await listener.accept();

println!(await client.write([104, 105]));
println!(await server.read(2));

client.close();
server.close();
listener.close();
```

Save it as `main.rnx` in a project and run `rnx run main.rnx`. It prints:

```text
true
2
[104, 105]
```

Port `0` asks the OS for a free port, and `port()` reads it back. `write()`
returns the number of bytes sent. `read(2)` returns whatever is available, up
to two bytes, so the result is an array. Close every socket when you are done
with it.

The two ends run in one process here. A real client or server uses the same
calls, with the peer on another machine or in another program.

## Keep going

Read [TCP sockets](guides/tcp) next for the read loop, framing, and the rules
that apply when one thread waits on another.
