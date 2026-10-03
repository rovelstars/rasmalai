# CLI

The `rnx` binary and its integration test suites.

## Subcommands (`src/args.rs`)

`dev` (hot reload), `run`, `build`, `check`, `test`, `bench`, `fmt`,
`lint`, `explain`, `init`, `add`, `fetch`, `lock`, `vendor`, `audit`,
`doc`, `pack`, `unpack`, `publish`, `completions`, plus LSP server
(`lsp/`), MCP server (`mcp.rs`), REPL, file watcher, and telemetry
supporting modules.

## Tests (`tests/`, 117 suites)

Each suite shells out to a debug-built `rnx` and pins user-visible
behavior end to end: language semantics (`operators_inc_dec`,
`string_ord`, `array_abi`), backends and linking (`release_build`,
`foreign_abi`), projects and registries (`workspaces`, `deplock`,
`project_init`), and safety (`security_runtime`). Fixtures live in
`tests/fixtures/`; temp projects go to the OS temp dir, never the repo.

`workspace_integrity.rs` enforces two repo invariants: core crates keep
zero external dependencies (allowlist with justification), and every
diagnostic code is both registered and referenced. `wave_3_doc_truth.rs`
keeps the website docs honest by executing what they promise.
