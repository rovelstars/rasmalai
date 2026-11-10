# @std/net

TCP sockets, TLS sessions, DNS lookups, and HTTP message shaping.

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

Read the [overview](/docs/@std/net/overview) or open the [API reference](/docs/@std/net/api).
