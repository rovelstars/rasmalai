# Contributing

## Test filesystem isolation

Every test that writes to disk must use a unique per-test directory. Shared
or hardcoded paths (`/tmp/…`, `target/…`, fixture trees, the package
directory itself) collide when tests run in parallel and leave litter behind
when a test panics.

Pull the helper in at the crate root of the test target:

```rust
#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;
```

Then build the test tree under it and point the subject at that directory:

```rust
let dir = crate::scratch::dir("mytag");
std::fs::write(dir.join("src/main.rnx"), "...").unwrap();
let out = std::process::Command::new(rnx())
    .arg("check")
    .current_dir(&dir)
    .output()
    .unwrap();
```

Rules:

- The `tag` names the test for triage (`ls $TMPDIR | grep rnx-test-…`).
  Uniqueness comes from the helper, not the tag, so reusing a tag is safe.
- The returned guard deletes the whole tree on drop, including on panic.
  Do not add manual `remove_dir_all` calls; they only race the guard.
- Never write outside the scratch directory. Fixture `.rnx` sources are
  read-only inputs; any file the compiler or the program under test creates
  goes under `dir`.
- Website tests assert on path strings (capabilities, permissions) rather
  than touching disk, so they need no tempdirs; if that changes, use
  `fs.mkdtemp` per test file, never a fixed path.

The helper is backed by the `tempfile` crate (a dev-dependency of every
crate whose tests include it). The random suffix is what keeps concurrent
runs apart; the `rnx-test-{tag}-{pid}-` prefix is only for humans.

## Running tests

Install the pinned runner once (`cargo install cargo-nextest --version 0.9.149 --locked`),
then mirror CI locally:

```sh
cargo nextest run --profile ci --workspace
```

`.config/nextest.toml` holds the `ci` profile: no fail-fast (every test
runs, as with the old `--no-fail-fast`), a slow-timeout that flags a test
at 15 minutes and terminates it a period later, and zero retries except
for the one quarantined wall-clock flake named in the file. Do not add
retries for other tests; a retry that masks a real failure is worse than
the flake it hides.

## Affected-only runs

`.github/scripts/affected-tests.sh` maps `git diff --name-only` against the
base ref to workspace packages via `cargo metadata`, follows reverse
dependencies, and prints a `-p …` list for nextest, or `FULL` when the
change touches anything outside `compiler/` or the mapping fails for any
reason. It fails open on purpose: unknown input runs everything, never
nothing. CI uses it on pull requests and runs the full suite on `main`:

```sh
FILTER="$(bash .github/scripts/affected-tests.sh origin/main)"
[ "$FILTER" = FULL ] && cargo nextest run --workspace --profile ci \
  || cargo nextest run --profile ci $FILTER
```
