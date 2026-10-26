# Standard library

The Rasmalai standard library, written in Rasmalai: 17 packages in
per-library folders (`src/<name>/lib.rnx`, plus `src/net/http.rnx`),
embedded into the compiler binary with `include_str!` (`src/lib.rs`) and
shipped with every program — no install step, no version skew.

Modules: `prelude`, `collections`, `math`, `simd`, `bytes`, `fs`,
`process`, `net`, `env`, `os`, `time`, `random`, `sync`, `json`, `web`,
`testing`, `io`.

Each folder carries a `docs/` directory (`overview.md`,
`getting-started.md`) that ships with the package as registry guides on
the next publish. The `@std/<name>` specifiers stay fixed no matter how
the folders are arranged: `src/lib.rs` is the single specifier-to-file
map, pinned by `stdlib_parity.rs`.

Each module is plain `.rnx` on top of the native runtime (`runtime/`) and
foreign C functions where the OS interface demands it. The documented
module list is pinned by `wave_3_doc_truth.rs` against the manual and the
website's stdlib index — adding a module means updating both docs.
