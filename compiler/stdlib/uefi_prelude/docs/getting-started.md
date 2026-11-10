# Getting started with @uefi/prelude

Select the prelude from a project with an alias:

```rnx
export default {
    project: {
        name: "uefi-probe",
        version: "0.1.0",
        targets: ["x86_64-unknown-uefi"]
    },
    dependencies: {
        "@std/prelude": { alias: "@uefi/prelude" },
        "@uefi/prelude": { path: "../../compiler/stdlib/uefi_prelude" }
    }
}
```

Programs see the foundation names, `Date`, and `println!`. Build them with
`rnx build src/main.rnx --target x86_64-unknown-uefi --release`.

```rnx
fn Main(): Int {
    let xs: Array<Int> = [1, 2, 3];
    println!("count", xs.len());
    return 0;
}
```
