# Standard library

The Rasmalai standard library, written in Rasmalai: 16 `.rnx` modules
embedded into the compiler binary with `include_str!` (`src/lib.rs`) and
shipped with every program — no install step, no version skew.

Modules: `prelude`, `collections`, `math`, `simd`, `bytes`, `fs`,
`process`, `net`, `env`, `os`, `time`, `random`, `sync`, `json`, `web`,
`testing`.

Each module is plain `.rnx` on top of the native runtime (`runtime/`) and
foreign C functions where the OS interface demands it. The documented
module list is pinned by `wave_3_doc_truth.rs` against the manual and the
website's stdlib index — adding a module means updating both docs.
