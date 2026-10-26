# @std/net

TCP sockets, TLS client sessions, and DNS resolution over a background
reactor.

Four types with one job each: `Dns` turns a name into an address,
`TcpStream.connect()` opens a connection, `TcpListener` waits for one, and
`TlsStream` upgrades a connected stream to TLS. Importing the module
opens nothing and reads nothing.

```rnx
import { TcpListener } from "@std/net";

let listener = TcpListener.bind("127.0.0.1", 0);
print(listener.port() > 0);
listener.close();
```

## What is inside

- `TcpStream`
- `TcpListener`
- `TlsStream`
- `Dns`

## When to use

Reach for @std/net when a program touches the network directly: TcpStream.connect for clients, TcpListener.bind plus accept for servers, TlsStream for encrypted sessions, Dns.lookup for resolution. Every operation parks in the kernel until ready; nothing spins.

It covers: TcpStream connect/read/write; TcpListener bind/accept; TlsStream sessions; Dns.lookup.
