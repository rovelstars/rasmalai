---
title: "Lexicon and Structure"
description: "Source encoding, comments, whitespace, statements, identifiers, and reserved keywords."
section: "Syntax and Primitives"
icon: "Type"
---

# Lexicon and Structure

## Source encoding

Source files are UTF-8. Non-ASCII bytes are permitted in string literals and comments. Identifiers are ASCII-only: a non-ASCII byte in identifier position is an `E108` error.

## Comments

Three comment forms exist. `//` starts a line comment. `//!` at the top of a file documents the module. `/* */` delimits a block comment; `/** */` documents the item that follows it. An unterminated block comment is an `E108` error. Plain `//` remarks never attach to items for documentation purposes.

```rnx
//! Module documentation.
//! Leading `*` markers inside doc blocks are stripped.

/** Scale a quote by demand pressure. */
fn quote(base: Float): Float {
    // Gutter remarks document nothing.
    return base;
}

print(quote(2.0));
```

### Documenting libraries with JSDoc

Library developers document with `/** */` blocks and `//!` module lines — the same shapes above, plus `@tags` the tooling understands. A doc block attaches to the item that follows it (functions, interfaces, and their members); each leading `*` is stripped. `/**/` attaches to nothing, and `///` is a plain comment despite the familiar look.

```rnx
//! Pricing helpers for the order book.

/**
 * Scale a quote by demand pressure.
 *
 * @param base The unadjusted quote.
 * @returns The scaled quote, always finite.
 * @throws When pressure is negative.
 *
 * @example
 * quote(2.0)
 */
fn quote(base: Float): Float {
    return base;
}
```

A line starting with `@` plus a letter opens a tag; `@param` takes the parameter name first, everything else after the tag name is prose (continuation lines join it). `@returns` (or `@return`), `@throws` (or `@error`), and `@example` render as labeled sections in `rnx doc`; any other `@tag` renders generically with its name. These docs feed the API reference: `rnx doc` embeds them per module, and published packages serve them from the registry for rendering and editor hovers.

## Whitespace and statements

Statements terminate at newlines and `}` boundaries; no semicolon separator is required. A lone `;` is a valid empty statement and is permitted anywhere a statement is permitted, including as a loop body or between items.

```rnx
;
let x = 1;;
print(x);
```

Blocks use curly braces. Indentation carries no meaning.

## Top-level scripts

The entry file (the `Project.config` entry, or the file passed to `rnx run`) may hold loose statements at file scope with no `fn Main()` wrapper. They run in lexical order inside a synthesized `async fn Main(): Promise<Int>`, so top-level `await` works and a top-level `return <Int>` sets the process exit code (falling through returns `0`). An explicit `fn main()`/`fn Main()` keeps the old behavior; combining it with loose statements is an `E108` error. Files reached via `import` stay declarative-only: a loose statement there is `E112`. That includes library entry points — when another package imports yours, your entry file is an imported module, so keep execution logic inside functions.

```rnx
let data = await Promise.resolve(42);
print(data + 1);
if data > 100 {
    return 1;
}
```

The ambient `rnx` object needs no import: `rnx.version` is the compiler version string, `rnx.args` holds CLI arguments after the script path, `rnx.cwd()` reads the working directory, and `rnx.exit(code)` terminates immediately. A local binding named `rnx` shadows it.

## Identifiers

An identifier starts with an ASCII letter or `_`, followed by ASCII letters, digits, or `_`. Identifiers are case-sensitive. Type names conventionally start with an uppercase letter; value names start with lowercase. The convention is unenforced except where stated (diagnostic `E303` applies to lowercase-led value names only).

## Reserved keywords

The following words are reserved and cannot serve as identifiers: `true`, `false`, `null`, `this`, `new`, `class`, `struct`, `record`, `trait`, `interface`, `extension`, `enum`, `fn`, `init`, `deinit`, `onReload`, `extends`, `with`, `let`, `const`, `static`, `if`, `else`, `for`, `while`, `in`, `return`, `break`, `continue`, `try`, `catch`, `finally`, `throw`, `throws`, `defer`, `guard`, `switch`, `case`, `default`, `do`, `fallthrough`, `is`, `import`, `from`, `as`, `public`, `pub`, `private`, `unsafe`, `comptime`, `native`, `async`, `await`, `pass`.

There is no `mut` keyword (`let` bindings are mutable). Classes construct with the `new` keyword (`new Meter(20)`); structs keep a direct call. There is no `::` path separator; a literal `::` is a compile error.

## Strings and text

`"..."` literals hold UTF-8 text as immortal static objects: string literals are never freed. `+` concatenates; `==` and `!=` compare by value. `<`, `<=`, `>`, and `>=` order lexicographically by unsigned byte value, with the shorter prefix sorting first (`"foo" < "foobar"`). Any `{expr}` or `${expr}` inside a string evaluates inline:

```rnx
let name = "rasmalai";
let count = 7;
print("Hello {name}! Count is ${count}.");
print("Hello " + name + "!");
```

Strings expose a fixed method set operating on character offsets: `length()` (also available as the `length` property), `slice(start, end)`, `indexOf(needle)`, `trim()`, `concat(other)`, and `charCodeAt(i)`. The prelude adds `contains`, `startsWith`, `endsWith`, `split`, `replace`, `replaceAll`, `repeat`, `toUpperCase`, and `toLowerCase` with no import.

```rnx
let text = "  hello world  ";
assert(text.trim() == "hello world", "trim");
assert(text.indexOf("world") == 8, "index");
assert("abc".charCodeAt(1) == 98, "char");
assert("hello".length() == 5, "len");
assert("foo bar".replace("bar", "baz") == "foo baz", "replace");
```

## Truth values

`Bool` has exactly two values, `true` and `false`. Conditions in `if`, `while`, and `assert` require `Bool`; `if 1 { }` does not compile.

```rnx
let ready = true;
if ready {
    print("go");
}
```

## Summary

- UTF-8 sources; ASCII-only identifiers; `//`, `//!`, `/* */`, `/** */` comments.
- Newline-terminated statements; lone `;` is an empty statement.
- Fixed keyword set; no `mut`, no `::` (`new` constructs classes).
- Immutable string literals with `{expr}` (or equivalently `${expr}`) interpolation and value equality. A literal brace is `\{`.
- Conditions require `Bool`.
