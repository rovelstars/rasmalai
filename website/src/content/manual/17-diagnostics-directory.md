---
title: "Diagnostics Directory"
description: "Every compiler diagnostic code: causes, examples, and resolution patterns."
---

# Diagnostics Directory

## Reading a diagnostic

Every message follows one shape: a short code, a plain sentence, a pointer at the exact column, and a fix hint. Reference to an undeclared name produces:

```text
┌─ error[E303]: undefined variable `nosuchvar`
│  src/main.rnx:1:26
│
│   1 │  fn main(): Int { let x = nosuchvar + 1; return x; }
·     │                           ▲
│
└─ note: declare the variable with `let` or check the spelling
```

Inside-out, the box states:

1. **The code** — `E303`. Codes starting with `E` are errors (exit 1); `W` codes are warnings (exit 0 unless `--deny-warnings`). Codes are stable across releases: they are searchable on this page, expandable with `rnx explain E303`, and matchable in CI with `rnx check --json`.
2. **The message** — one sentence naming the problem in program terms, never compiler internals.
3. **The location** — file, line, and column, with a source excerpt and a `▲` pointer under the exact column.
4. **The note** — the fix: what to type next.

Warnings print in yellow without failing the build. Fatals (`fatal: ...`) report runtime stops — traps and failed assertions — in the same box shape with the function and span where execution stopped. Uncaught throws print as `Uncaught exception: <value>` on stderr with a `Stack trace:` section naming the stop site instead of a box. Errors stop the pipeline; when several appear, the first is fixed first, since later errors are frequently downstream echoes.

`rnx explain` expands any code into full text plus fix:

```sh
rnx explain E303
```

```text
E303: undefined variable
fix: declare the variable with `let` or check the spelling
```

## Lexical codes (E005)

| Code | Severity | Cause | Resolution |
|---|---|---|---|
| `E005` | error | removed `::` syntax (paths, variants, generics use `.` and `<>`) | replace `::` with `.` |

## Module and declaration codes (E105-E112)

| Code | Severity | Cause | Resolution |
|---|---|---|---|
| `E105` | error | `fn(...) => ...` lambda form; `fn` declares named functions with block bodies only | use `(params) => body` for lambdas |
| `E107` | error | circular package dependency; reserved for external package graphs, never emitted for intra-project file imports | break the dependency cycle between packages |
| `E108` | error | general compile error: single-statement `if` bodies holding declarations, `pub` signatures with non-C-compatible types, missing operator hooks, unknown packages, and related structural violations | read the message; it names the exact rule (declarations need `{ ... }`, hooks name the operator and type) |
| `E109` | error | `await` outside an `async fn` (entry-file top level is exempt) | move the `await` into an `async fn` or to the entry file top level |
| `E110` | error | C-style `for (init; cond; step)` | use `for x in range` or `while` |
| `E111` | error | direct `.poll()` call; the polling model is retired | use `await` or `promise.wait()` instead |
| `E112` | error | loose top-level statement or top-level `await` in an imported module | move execution logic into a function or class method; top-level statements live in the entry file only |

The `E108` declaration rule, corrected:

```rnx
let x = 1;
if x > 0 {
    let y = 2;
    print(y);
}
```

## Boundary codes (E201-E204)

| Code | Severity | Cause | Resolution |
|---|---|---|---|
| `E201` | error | call to an `unsafe fn` outside an `unsafe` block | wrap the call in `unsafe { ... }` |
| `E202` | error | address-of (`&x`) or `Pointer` arithmetic outside `unsafe` | move the operation into an `unsafe` block |
| `E203` | error | `private` field, method, or module item accessed outside its scope | widen visibility or access through the owning scope |
| `E204` | error | class invoked as a direct call (`Meter(20)`) instead of `new` | instantiate with `new Meter(20)` |

```rnx
class Meter {
    let v: Int;
    init(v: Int) { this.v = v; }
}

let m = Meter(20);
```

The direct call fails with `E204`; `new Meter(20)` is the fix.

## Binding codes (E205-E206)

| Code | Severity | Cause | Resolution |
|---|---|---|---|
| `E205` | error | assignment type mismatch: the value type is not assignable to the variable type | declare the variable as `Any` for dynamic reassignment, or use `T?` for nullable bindings |
| `E206` | error | `let`/`const` without an initializer | add `= <expr>` (reported instead of `E108`, with recovery on `;`) |

```rnx
let count: Int = 0;
count = "forty-two";
```

The reassignment fails with `E205`: a `String` is not assignable to an `Int` binding. Declaring the variable as `Any` is the fix.

```rnx
let count: Any = 0;
count = "forty-two";
```

A `const` declared without an initializer fails with `E206`.

```rnx
const limit: Int;
```

Adding the initializer is the fix.

```rnx
const limit: Int = 10;
```

## Type and ownership codes (E302-E305)

| Code | Severity | Cause | Resolution |
|---|---|---|---|
| `E302` | error | intra-procedural ARC cycle without escape | restructure so the value escapes, or route the back-edge through `GenRef` |
| `E303` | error | reference to a value name with no visible declaration; applies to lowercase-led names only (`PascalCase` names are type references that single-file checks leave alone) | declare with `let` or correct the spelling |
| `E304` | error | `return` value does not match the declared return type, with `expected`/`got` labels | align the returned value with the signature |
| `E305` | error | implicit `Float`/`FastFloat` mix in one expression | convert explicitly with `.asFast()`/`.asStrict()` |

```rnx
let strict: Float = 0.5;
let fast = strict.asFast();
print(fast * 2.0.asFast());
```

## Packaging codes (E402, E501)

| Code | Severity | Cause | Resolution |
|---|---|---|---|
| `E402` | error | schema mismatch in a typed `file:` import | align the import schema with the file |
| `E501` | error | publish attempted without a publisher token | pass `--token <token>` or set `RNX_TOKEN` |

## Security codes (S101-S501)

Capability and permission failures. The full model — capability grammar, `[permissions]` ceilings, tiers, and the `rnx audit` / `rnx lock` workflow — is specified in [Project and Toolchain](/manual/16-project-and-toolchain).

| Code | Severity | Cause | Resolution |
|---|---|---|---|
| `S101` | error | capability mismatch: deduced capabilities exceed the `Project.deplock` ledger | approve the capability and re-lock, or remove the offending use |
| `S102` | error | permission ceiling exceeded: a capability outside `[permissions] allowed` | narrow the package code or deliberately widen the ceiling |
| `S201` | error | untrusted path mutation: a delegated argument modified before reaching a sink | pass the delegated value through untouched or request the ambient grant |
| `S301` | runtime trap | protected path violation: dependency code targeting `Project.config`, `Project.deplock`, `.git`, or cache dirs | never target those paths from dependency code; the abort is the intended enforcement |
| `S401` | runtime trap | unapproved child process execution | add a covering `sys:exec:<name>` grant with explicit approval |
| `S501` | error | capability analysis timeout | split the package into smaller modules; never skip the scan |

## Cycle warnings (W104-W109)

| Code | Severity | Cause | Resolution |
|---|---|---|---|
| `W104` | warning | self-capturing closure without decay | capture `GenRef(this)` with `fn decay(this)` |
| `W108` | warning | mutual strong fields `A<->B` | convert one side to `GenRef` |
| `W109` | warning | `#[Allow(CyclicReference)]` field never cleared to `null` | clear the field in `deinit` or a `clear*`/`close*`/`reset*` method |

## Manifest warnings (W201)

| Code | Severity | Cause | Resolution |
|---|---|---|---|
| `W201` | warning | unrecognized manifest section or field; ignored | correct the spelling or remove the entry |

## Lint rules (L001-L005)

`rnx lint` reports these as `warning[L00N]` findings; `--deny-warnings` promotes them to build failures for CI.

| Code | Meaning |
|---|---|
| `L001` | `let`/`const` never read (`_` prefix silences) |
| `L002` | function or method parameter never used (`_` prefix or empty trait methods exempt) |
| `L003` | statements after `return`, `throw`, `break`, or `continue` |
| `L004` | `pub` fn/class/struct/trait/enum without `/** */` docs |
| `L005` | empty `{}` in `if`, `while`, `defer`, `try`, or `unsafe` blocks |

## Summary

- Every diagnostic is code plus sentence plus `▲` pointer plus fix note; codes are stable and CI-matchable.
- Errors fail the build, warnings advise, fatals report runtime stops.
- `rnx explain <code>`, editor hovers, and `rnx check --json` expose the same registry to humans and scripts. `explain` covers every `E`, `W`, and `S` code; the `L001`-`L005` lint rules below are documented in their table and fixed per row rather than through `explain`.
