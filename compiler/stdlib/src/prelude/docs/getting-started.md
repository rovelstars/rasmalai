# Getting started with @std/prelude

No install step: the module ships inside the compiler.

The prelude is already in scope in every file, no import needed.

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
// no import: the prelude is ambient

println!("no import needed");
let found = [1, 2, 3].find((n) => n > 1);
assert("abc".contains("b"), "prelude");
println!(found ?? -1, typeOf(found));
```

## Pick another prelude

Alias `@std/prelude` to a package that re-exports the catalog without the
host console and logging surface:

```rnx
export default {
    project: { name: "uefi-probe", version: "0.1.0" },
    dependencies: {
        "@std/prelude": { alias: "@uefi/prelude" },
        "@uefi/prelude": { path: "../../compiler/stdlib/uefi_prelude" },
    },
}
```

The selected package must declare the required names; a missing name fails
`E108` with the package and the name.

## Keep going

Read the [overview](overview) for the full picture, then open the API
reference for exact signatures.
