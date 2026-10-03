# Diagnostics

The shared error system. Every user-facing error in every stage carries a
stable code; docs, the `rnx explain` command, and the website all key off
this registry.

## Files

- `lib.rs` — `Span` (byte offsets), the `Code` enum (`E...` errors,
  `W...` warnings, `ALL` registry with titles for round-trip checks),
  and `Diagnostic` (code + message + optional span).
- `theme.rs` — terminal rendering: colors, underlines, multi-span layout.
- `tree_hook.rs` — structured hooks for machine-readable consumers.

Conventions: codes are never reused or renumbered; new codes need a title,
an `explain` fix entry in `cli`, and at least one reference in compiler
sources or tests (`workspace_integrity.rs` enforces the last two).
