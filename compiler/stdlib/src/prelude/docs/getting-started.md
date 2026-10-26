# Getting started with @std/prelude

No install step: the module ships inside the compiler. Import what you need:

The prelude is already in scope in every file, no import needed.

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
// no import: the prelude is ambient

print("no import needed");
let found = [1, 2, 3].find((n) => n > 1);
assert("abc".contains("b"), "prelude");
print(found.unwrap(), typeOf(found));
```

## Keep going

Read the [overview](overview) for the full picture, then open the API reference for exact signatures.
