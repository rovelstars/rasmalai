# Getting started with @std/net

No install step: the module ships inside the compiler. Import what you need:

```rnx
import { TcpListener } from "@std/net";
```

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
import { TcpListener } from "@std/net";

let listener = TcpListener.bind("127.0.0.1", 0);
print(listener.port() > 0);
listener.close();
```

## Keep going

Read the [overview](overview) for the full picture, then open the API reference for exact signatures.
