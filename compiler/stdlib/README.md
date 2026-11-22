# Standard library

The Rasmalai standard library, written in Rasmalai: 25 packages in
per-library folders (`src/<name>/lib.rnx`, plus `src/net/http.rnx`),
embedded into the compiler binary with `include_str!` (`src/lib.rs`) and
shipped with every program. No install step, no version skew.

Modules: `prelude`, `foundation`, `async`, `simd`, `math`, `collections`,
`config`, `fs`, `bytes`, `time`, `random`, `sync`, `env`, `process`, `os`,
`testing`, `web`, `json`, `net`, `io`, `log`, `uefi`, `wasi`, `ui`, `app`.

Each folder carries a `docs/` directory (`overview.md`,
`getting-started.md`) that ships with the package as registry guides on
the next publish. The `@std/<name>` specifiers stay fixed no matter how
the folders are arranged: `src/lib.rs` is the single specifier-to-file
map, pinned by `stdlib_parity.rs`.

Each module is plain `.rnx` on top of the native runtime (`runtime/`) and
foreign C functions where the OS interface demands it. The documented
module list is pinned by `wave_3_doc_truth.rs` against the manual and the
website's stdlib index, so adding a module means updating both docs.
