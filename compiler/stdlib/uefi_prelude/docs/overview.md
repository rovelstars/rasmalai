# @uefi/prelude

The target prelude for `x86_64-unknown-uefi` programs.

It re-exports the foundation catalog from `@std/foundation`, `Date` from
`@std/time`, and the console surface from `@std/uefi`. There is no
`@std/io`, `@std/log`, `@std/fs`, or `@std/process`, so `println!` is the
UEFI one and the host stream types are absent.

```rnx
fn Main(): Int {
    println!("hello from uefi");
    return 0;
}
```

`IO`, `Thread`, and `ThreadPool` are not in scope. A program that needs
them cannot target UEFI.
