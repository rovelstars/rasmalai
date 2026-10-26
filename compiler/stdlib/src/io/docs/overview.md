# @std/io

Terminal streams, pretty-printing, line input, and raw mode.

`io` owns the terminal: standard streams, color-aware pretty-printing,
line input, size queries, and raw mode. File I/O stays in `@std/fs`.
Every item uses the `export` keyword. There is no ambient `Console`
class. `print` stays in the prelude as a shim over `io.write`.

```rnx
import io from "@std/io";

io.write("hello");
print(io.width() > 0);
```

## What is inside

- `ColorLevel`
- free functions: `stdin()`, `stdout()`, `stderr()`, `write()`, `writeRaw()`, `writeError()`, `clear()`, `read()`

## When to use

Reach for @std/io when a program talks to the terminal: write and writeError for color-aware pretty-printing with newlines, writeRaw for exact bytes, read and readLine for input, width and height for layout, colorProfile for the ColorLevel tier, setRawMode for character-at-a-time input. File I/O stays in @std/fs.

It covers: Color-aware pretty-printing; stdin/stdout/stderr handles; Line input and size queries; Raw mode toggling.
