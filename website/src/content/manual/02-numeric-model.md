---
title: "Numeric Model"
description: "Unified 64-bit semantics: integers, floats, bitwise ops, and conversions."
---

# Numeric Model

Rasmalai has no platform-dependent arithmetic. Every numeric type has a fixed width on all four backends (interpreter, Cranelift JIT, LLVM, AOT). `Int` and strict `Float` are additionally bit-identical across backends; `FastFloat` permits reassociation, so its last bits may differ between backends. This chapter states them exactly.

## Integers: one width

`Int` is a 64-bit two's-complement signed integer on every platform. There are no sized integer types for general use (`u8`, `i32`, and friends do not exist in the language surface).

- **Overflow wraps** modulo 2^64. There is no overflow trap and no undefined behavior.
- **Division and remainder by zero are runtime fatals.** The program aborts with a diagnostic; it never produces a garbage value.
- **Literals**: decimal (`42`), hex (`0xFF`), binary (`0b1010`), with `_` digit separators (`9_000`). All are plain `Int`.
- **Conversions**: `Int(x)` truncates a `Float` toward zero. `Int` operands inside `Float` arithmetic convert implicitly.

```rnx
print(17 / 5, 17 % 5);
print(0xFF_00, 0b1010);
print(Int(3.9), Int(-3.9));
```

## Bitwise operators

`&`, `|`, `^`, and unary `~` operate on `Int` with C-style precedence (`~` tightest, then `&`, `^`, `|`, all above equality). Combined with hex literals they express bit patterns directly:

```rnx
let flags = 0b1010;
print(flags & 0b0010);
print(flags | 0b0101);
print(flags ^ 0b1111);
print(~0 & 0xFF);
```

Shifts work on `Int`: `<<` (left), `>>` (arithmetic right, sign-extending), `>>>` (logical right, zero-fill). They bind tighter than comparisons but looser than `+`/`-`, and the shift amount masks to 6 bits (`1 << 67` shifts by 3). Compound forms `<<=`, `>>=`, `>>>=` update in place:

```rnx
print(1 << 3);
print((-8) >> 2);
print((-1) >>> 1);
```

## Floats: strict by default, fast by consent

`Float` is IEEE-754 double precision with strict evaluation order: `0.1 + 0.2` behaves exactly as the standard says, identically on every backend. `FastFloat` is the same 64 bits with the optimizer permitted to reassociate — which can change the last bit of a result.

The type system enforces the distinction: mixing `Float` and `FastFloat` in one expression without an explicit conversion is a compile error.

- `.asFast()` / `.asStrict()` are zero-cost marker conversions. They change the type rules, not the bits.
- `Float(x)` converts an `Int` to `Float`.
- Scientific notation needs no decimal point: `1e16`, `2E+8`, and `5e-3` are `Float` literals, exactly as written.
- `Math.sqrt`, `Math.sin`, `Math.floor`, `Math.log`, and related intrinsics take and return `Float`; negative `sqrt` input yields NaN per IEEE-754, `Math.log(0.0)` yields negative infinity, and `Math.log` of a negative input yields NaN.
- NaN is native IEEE-754 hardware NaN: sign and payload bits are implementation-defined and may differ between backends, matching C and Rust. `Float.isNaN(x)` tests for it; `x != x` is also true for NaN. (`Float.nan()` constructs the canonical `0x7FF8000000000000`.)
- The compiler never fuses `a * b + c` on its own for `Float`. For
  `FastFloat`, a multiply whose only use feeds an addition fuses into a
  single-rounding hardware FMA on every backend — but reassociation means
  the backends may fuse *different* multiplies (LLVM can strength-reduce
  `x + x` to `2 * x` first, Cranelift does not), so `FastFloat` results
  can differ in the last bits between dev and release. Only `Float` is
  reproducible. Spelled-out fusion is `Float.fma(a, b, c)`, one rounding,
  no intermediate overflow.
- Bit accessors: `Float.nan()` returns canonical NaN, `x.toBits()` reinterprets a `Float` as `Int` bits, `Float.fromBits(bits)` builds a `Float` back with bits preserved exactly.

```rnx
import { Math } from "@std/math";

let strict: Float = 0.5;
let fast = strict.asFast();
print(fast * 2.0.asFast());
print(Math.sqrt(2.0) > 1.4);
print(Float(7) / 2.0);
```

## Interop lanes

`ByteBuffer` (`@std/bytes`) exposes the bit-level view: integer lanes read as 64-bit `Int` (signed lanes sign-extend, unsigned lanes zero-extend) and float lanes read as 64-bit `Float`. Out-of-bounds lane access aborts. This is the sanctioned path for float bitcasting and wire formats — see [Hardware and FFI](/manual/13-hardware-and-ffi).

```rnx
import { ByteBuffer } from "@std/bytes";

let buf = ByteBuffer.allocate(16);
buf.writeFloat64BE(0, 1.5);
print(buf.readFloat64BE(0));
buf.writeInt32BE(8, 0x01020304);
print(buf.readInt32BE(8));
```

## Summary

- `Int`: 64-bit, wrapping overflow, fatal divide-by-zero, `Int(x)` truncates.
- `Float`: strict IEEE-754 double. `FastFloat`: reassociation allowed, explicit `.asFast()` boundary required.
- Bitwise ops on `Int` with C precedence; literal bases `0x`/`0b` with `_` separators.
- Bit-level reinterpretation goes through `ByteBuffer` lanes.
