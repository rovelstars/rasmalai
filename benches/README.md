# Rasmalai benchmark suite

One runner lives in this directory:

- `harness/runner.py`: the unified multi-language suite (`compute/`,
  `memory/`) across rnx, C, Rust, Go, Node.js, Java, and Dart, emitting
  `data/benchmarks.json` for the website Pareto chart. Timing flows
  through the C helper (`harness/measure.c` via `measure.sh`), a single
  code path for all languages.

Each workload prints `RESULT <key> <values>` lines; runners check
checksums across configurations and report wall time with peak RSS.

## One point per (language, mode)

A row in `benchmarks.json` is a single configuration, not a single
language. Languages whose toolchain has a real dev/release switch are
measured twice, so `rnx dev` and `rnx release` can be read against each
other directly on the chart:

| language | dev | rel |
| --- | --- | --- |
| rnx | `rnx build` (LLVM AOT dev: unoptimized object + linked runnable binary) | `rnx build --release` (LLVM `-O3` AOT) |
| c | `clang -O0` | `clang -O3 -march=native` |
| rust | `rustc` (unoptimized) | `rustc -C opt-level=3 -C codegen-units=1` |
| dart | `dart compile kernel` (JIT snapshot) | `dart compile exe` (AOT native) |
| go | - | `go build` only: Go always compiles optimized |
| node | - | interpreted, no build step; the timed step is a parse check |
| java | - | `javac`, which has no `-O` flag; HotSpot tiers its own JIT |

The three single-mode languages are not padded with synthetic flags. Each
row records its own `build_ms` (median of 3 builds after 1 warm-up), so
the chart's y axis is per-point: a dev dot sits low and slow, its release
sibling sits high and fast, and that gap is the language's own dev/rel
tradeoff.

`build_ms` is 0 with `artifact: false` only when a toolchain genuinely has
nothing to time; both the runner and the website benchmark ingest API
reject a zero build time on a row that claims to produce an artifact.

## Unified suite layout

```
benches/
  harness/
    runner.py    # builds, measures (median of 5 + 1 warm-up), validates, emits JSON
    measure.sh   # standalone wall-time + peak-RSS wrapper for one command
  data/
    benchmarks.json  # generated, gitignored: {system, versions, benchmarks[]}
                     # (monthly CI publishes it to the orphan `benchmarks` branch)
  compute/
    fib/ mandelbrot/ nbody/ spectral/ matmul/
  memory/
    binary_trees/ json_stress/
```

Each benchmark directory holds one idiomatic implementation per
language (`main.rnx`, `*.c`, `*.rs`, `*.go`, `*.js`, `*.java`, `*.dart`).
Fixed sizes: fib(35), mandelbrot 400x400 @ 1000 iterations, nbody 5
bodies / 100k steps (Benchmarks-Game units, energy printed, 1e-6
relative tolerance), spectral N=300 x 10 iterations (1e-6 tolerance),
matmul 256x256 integers, binary trees depth 16 in struct-of-arrays
form, json 20000 flat objects (~1.5 MB) x 5 parse+serialize rounds.

`python3 benches/harness/runner.py --dry-run` prints the plan
(toolchain detection, source matrix) without building. Full run:

```
python3 benches/harness/runner.py [--benches fib,matmul] [--langs rnx,c]
                                  [--runs 5] [--out benches/data/benchmarks.json]
```

Per language: release build + run (`rnx build --release`, `clang -O3
-march=native`, `rustc -C opt-level=3 -C codegen-units=1`, `go build`,
`node`, `javac` + `java`, `dart compile exe`). C float benches
(mandelbrot, nbody, spectral) add `-ffp-contract=off` so every language
evaluates strict IEEE-754: without it clang fuses `a*b+c` into FMA under
`-march=native` while rnx `Float`, rustc, Go, HotSpot, V8 and Dart all
evaluate strictly. One deliberate exception: the rnx mandelbrot
  source computes the orbit loop in `FastFloat` with LIR-synthesized
  single-rounding FMA (spec 04_NUMERICS section 3), so its integer
  checksum (27482298) differs from the strict double-rounding
  checksum every other language prints (27482318) - 20 pixels out of
  160k orbit on a different side of an escape boundary. The harness
  tracks both references (`rnx_variant` in `benchmarks.json`); all rnx
  backends agree on the FMA value bit-for-bit, all other languages
  agree on the strict value. Pixel coordinates stay strict `Float` in
  the rnx source so only the loop rounding differs. `build_ms` is benchmark source to runnable
artifact (`rnx build` dev profile: unoptimized LLVM object plus link, `-O0` /
unoptimized compiles, `javac`); Dart's dev step is a JIT snapshot
(`dart compile kernel`) rather than a dev binary, and Node has no compile
step at all, so it runs the `node --check` parse check - each result row
records its own `build` string, and the chart methodology blurb says which
cells are which.
The `rnx` compiler binary prefers `target/release/rnx` when
present (recorded in `rnx_compiler`); the runtime archive rnx links is
built at `-O3` (`compiler/runtime/build.rs`), program body at
LLVM `default<O3>` for the host CPU. Set `RNX_RUNTIME_TARGET_CPU=native`
(or pass `-C target-cpu=` via `RUSTFLAGS`) before building `rnx` to
compile the runtime archive for the host CPU as well; default is
portable generic x86-64. Every result row carries `build` and `run`
descriptors with the exact commands, surfaced on the website tooltip
and methodology list. Languages whose toolchain is missing are skipped
with a reason; Go/Java sources are provided for machines that have them.
Timing uses the compiled `harness/measure` helper (fork/exec plus
`wait4` from a ~16 KB parent, JSON on stdout); `measure.sh` builds it
on demand with `${CC:-cc}` and has no other code path, so the child's
`ru_maxrss` is the benchmark binary itself.

Comparability notes:

- External wall time includes process startup for every language
  (JVM/HotSpot and V8 startup, AOT process init). It matters most for
  the shortest benches (fib); all implementations still do the same
  timed work after startup.
- mandelbrot, nbody and spectral C are built with `-ffp-contract=off`:
  `-march=native` otherwise fuses FMA, changing results versus strict
  IEEE evaluation in every other language. (nbody/spectral checksums
  use a 1e-6 relative tolerance, which would hide the mode difference;
  the flag - not the tolerance - is what keeps the comparison honest.)
  One deliberate exception: the rnx nbody source uses `FastFloat`
  (spec 04_NUMERICS lists leapfrog integration under FastFloat use).
  Dropping per-op NaN canonicalization is what lets the rnx inner loop
  match C (16.3 ms down to ~7 ms); the integrated energy agrees with
  the strict spelling to ~1e-15 relative on every backend, inside the
  harness tolerance, so the row stays validated rather than
  specially-cased. Same for the rnx spectral source: `FastFloat`
  accumulation with LIR-synthesized FMA, bit-identical to its strict
  spelling on every backend, inside tolerance everywhere else.
- spectral `aElem` does integer division in every language (the
  dividend is always even, so exact); the JS spelling says so
  explicitly with `| 0`.
- binary_trees keeps the depth-16 long-lived tree alive through the
  loop in every language (Benchmarks-Game shape); growth policy is
  each language's default (C seeds 1024 and doubles, the rest use
  growable vectors/lists). Java uses unboxed `long[]` tables like the
  integer arrays everywhere else, not boxed `ArrayList<Long>`.
- json implementations differ in parser tier, honestly: rnx (streaming
  tape with interning), Go (`encoding/json` into typed structs), Node
  (V8), and Dart (`dart:convert`) use tuned paths, while C/Rust/Java
  hand-roll DOM parsers (realloc-per-item, duplicated key strings, no
  interning). Go decodes into typed `[]Obj` (no maps, fixed field
  order) where everyone else builds generic objects/maps. Same bytes
  in, same checksums out; the rows rank whole-stack JSON cost, not
  just language speed.
- The json fixture is ASCII-only (`id`, `name-7`, `a/b/c`, `1.5`,
  `true`). C/Rust/Java copy strings raw without escape handling,
  which is correct for this fixture only, not for general JSON.
- rnx `stringify` sorts keys; the rest preserve insertion order. The
  checksum is a byte sum, so it is order-insensitive by construction;
  byte-for-byte equality across languages is NOT asserted.
- rnx json renders the full output with one `JSON.stringifyInto` call
  into a reused buffer (same bytes as a one-shot `stringify`, same
  shape as the other languages' serializers) and folds the checksum
  over it, so no second string copy is ever live.
- rnx `Array` has no pre-sized allocation API, so nbody/spectral/matmul
  build working arrays with `push` (growth cost inside the timed
  region) where C uses BSS and Rust/Go/Java/Dart pre-size. Same
  element counts and flops.
- rnx strings interpolate `{expr}`: brace literals must be escaped as
  `\{`. `charCodeAt` in a hot loop is quadratic; fold checksums through
  a reused `ByteBuffer` window (`readInt64LE` lanes plus a `readUInt8`
  tail) instead.
- CPU-target asymmetry, documented not hidden: C builds with
  `-march=native` and the rnx program body with host CPU + features,
  while Rust (`rustc` without `-C target-cpu=native`) and Go
  (`GOAMD64=v1` baseline) compile for portable x86-64. C is the only
  peer that sees AVX2+ in its own code; the rnx runtime archive is
  generic x86-64 unless `RNX_RUNTIME_TARGET_CPU` is set at `rnx` build
  time (see above). Rows therefore compare portable-Rust/Go against
  host-tuned C/rnx-body - read cross-language gaps as lower bounds.
- nbody velocities are AU/day and must be scaled by 365.24; the sun's
  velocity comes from momentum offset. Step-0 energy is -0.169075164.

## Standalone workloads

Four `.rnx` programs at the top level, exercised directly by the
`benches_smoke` CLI test (`rnx run` at tiny scale on all three
backends, checksums must agree):

- `binary_trees.rnx`: bottom-up binary trees in struct-of-arrays form
  (three parallel `Array<Int>` tables, child = index, -1 = empty) plus
  flat object allocation loops. Checksums follow the Benchmarks Game
  shape (stretch, per-depth, long-lived); `live_delta` counts net new
  live allocations over the run.
- `parallel_workers.rnx`: `ThreadPool.parallelFor` reductions over 1,
  2, 4, and 8 workers with per-index `AtomicInt` slots. Checksums must
  agree across worker counts (`scaling_ok`).
- `net_throughput.rnx`: one server thread echoes 64-byte payloads; the
  client counts round trips through the mio reactor. `pingsum` is
  validated against an independent Python computation.
- `json_stress.rnx`: `JSON.parse` + `JSON.stringify` over a generated
  nested document (~1 MB at full scale, two-level build to avoid
  quadratic concat garbage). Canonical output must match the Node
  baseline byte for byte.

## Scales

`RNX_BENCH_SCALE` selects `tiny` (smoke, seconds), `small`, or `full`
(default). The `benches_smoke` CLI test runs every workload at tiny
scale on all three backends and asserts identical checksums.

## Reading the numbers

- `live_delta` means different things per backend: the interpreter
  reports live arena/table entries, the JIT backends report the
  `rnx_alloc` balance. Small constants are healthy; growth
  proportional to workload size points at retained temporaries.
- Peak RSS includes allocator arena retention (freed pages the C
  library keeps mapped), so it overstates steady-state use after
  allocation-heavy phases such as document building.

## Known runtime defects found while building this suite

The three ownership leaks below are fixed. The fixes live in
`lir::temp_sweep` (single-use fresh-value release insertion),
the LLVM/Cranelift call lowering (post-call `Any`-argument release
gated by callee analysis), and `runtime::native` (owned map keys,
streaming JSON parse, string interning).

1. Anonymous concat temporaries: chained `Concat`/`ToStr` products and
   discarded call results now get explicit `Release` instructions from
   the `temp_sweep` LIR pass (single-use fresh values only), so all
   backends stay balanced.
2. `Any`-argument call retains: the caller now emits `Release` after a
   `CallTarget::Fn` call returns, but only when the callee provably
   does not consume its parameters (no parameter flows into
   `release_box`/`json_unwrap`/`gmap_free`/`bytes_free`/
   `channel_drop`). Consuming callees keep the historical
   caller-retains/callee-frees balance.
3. Statement-boundary orphans: untyped temporaries produced by calls,
   boxing, and string conversion are released after their single
   consuming use (copy into an `Any` slot, concat operand, or a
   discarded result) instead of accumulating until frame exit.

Supporting runtime fixes (all in `crates/runtime/src/native.rs`):

- Map keys are owned, not borrowed: `set` retains the key,
  `delete`/`clear`/destroy release it. This was load-bearing before:
  the old call-site retain leak kept borrowed keys alive by accident.
- `JSON.parse` streams through the serde lexer directly into runtime
  values (no intermediate DOM), buffers object entries to preserve
  sorted key order, and interns repeated strings. Parse time more
  than halved; peak memory no longer carries a second copy.
- The `Any`-box registry is a flat open-addressing set instead of a
  hash set with per-entry nodes.
- Small maps (at most 8 entries) skip the hash index and scan
  linearly.

Historical note: the three items above replace the original defect
descriptions (anonymous `Any` temporaries orphaning interpreter table
entries, per-call `Any`-argument retains, unreleased concat call-result
temporaries). No other pre-existing issues are known in this area.
