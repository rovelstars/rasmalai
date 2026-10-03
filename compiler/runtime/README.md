# Runtime

Tree-walking interpreter plus the native C-ABI runtime every backend calls
into. The `rnx_*` symbol surface is frozen: the interpreter, Cranelift JIT,
and LLVM backend all resolve these names dynamically, so functions are
never renamed or removed, only added.

## Files

- `machine.rs` — the interpreter: executes verified LIR directly.
- `value.rs` — runtime value representation (tagged ints, floats, strings,
  heap objects, arrays, closures).
- `native/` — the `pub unsafe extern "C"` function surface, split by
  domain behind a re-exporting `mod.rs`:
  - `common.rs` — allocation, string headers, assertions.
  - `string.rs` — comparison, slicing, formatting.
  - `sync.rs` — threads, atomics, channels, mutexes, thread pools.
  - `collections.rs` — maps, arrays, bytes, JSON, `Any` boxing, closures.
  - `io.rs` — printing, files, env, processes, dynamic library loading.
  - `net.rs` — TCP, DNS, listeners, TLS handshakes.
  - `math.rs` — floats, PRNG.
- `threads.rs`, `reactor.rs` — OS threads and the `mio` event loop
  (epoll/kqueue/IOCP); `reactor_wasm.rs` is the browser stub.
- `tls.rs` — TLS sessions on `rustls`.
- `ichan.rs` — internal channels; `guard.rs` — stack-overflow guards.
- `json_scanner.rs`, `json_tape.rs` — streaming JSON document support.
- `build.rs`, `bundle/` — the build script recompiles `native/` as a
  standalone `runtime_native` static/shared library (`bundle/` holds only
  the helper manifest; all code lives in `src/`). The archive is embedded
  into the compiler (`src/lib.rs`) and linked into generated programs.
