# The UI core

Nodes, layout, hit testing, focus, keyboard activation, and the semantic walk.
Headless: no renderer, no window, no bus.

This is M0 from `requirements_UI/49-m0-contract.md`. The parts of the contract
that exist here are the arena, the builder, layout, input dispatch, focus, and
the semantic tree. The packet encoding and reconciliation come next.

## How it is built

**Parallel arrays.** `Tree` keeps one array per node property and addresses nodes
by integer id. A subtree walk is index arithmetic, a snapshot is a copy, and
there is no per-node allocation. Child counts are stored when a node is attached,
so `childCount` is O(1) instead of a sibling walk.

**Wire ids for roles and actions.** The arena stores `Int`, not the enum. An
enum-typed array in a class field currently trips the prelude interface check, and
an integer lane is smaller anyway. The enums stay in method signatures, which is
all an author sees.

**Classes for the public surface.** A `pub fn` with a non-C signature is rejected
unless the package declares a cabi file, so the core exports classes and enums
and keeps free helpers package-internal.

**A builder, not a callback.** Views receive a `Build` and emit `open`/`close`
pairs. The UI block syntax compiles to exactly these calls, so the sugar adds no
runtime concept.

**Two layout passes.** `measure` walks down with the width a node receives and
back up with the height it needs. `place` walks down once more and writes
rectangles. No allocation per pass, no layout tree.

## Try it

```sh
export RNX_VENDOR_DIR=$PWD/compiler/stdlib/src
cd compiler/stdlib/src/ui && rnx test
cd examples/ui-demo && rnx run --backend interpreter
```

## Not here yet

- Keyed reconciliation. Nodes are append-only for now.
- The frame packet. Layout, layout bounds, and the semantic tree are separate
  views in memory rather than one encoded buffer.
- A renderer. The CPU rasterizer is the next milestone.

## Regression checks

The `test fn` blocks cover creation order, deepest-node hit testing, tap dispatch,
Enter reaching the same handler, focus wraparound, column stacking, root
protection, and the unnamed-interactive audit.
