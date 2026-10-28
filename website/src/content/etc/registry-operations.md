---
title: "Registry Operations"
description: "Deploy the registry, configure it, run publishes, garbage collection, and advisories, and verify the deployment."
section: "Registry"
order: "6"
icon: "Wrench"
---

# Registry Operations

This page takes a stranger from zero to a running registry: what to deploy, which variables to set, how the database comes up, how each mutation works, and how to check the deployment afterward. It mirrors the spec in `requirements/15_REGISTRY.md`; where behavior differs, the spec governs and the gap is filed at the bottom.

## What runs where

The registry is the `website` project in this repo, deployed as one Cloudflare Pages project with Functions. Three stores back it:

- **Turso (libSQL over HTTP)** holds every row: packages, versions, manifests, advisories, audit log, download rollups. The worker talks to it through `TURSO_DATABASE_URL` and `TURSO_AUTH_TOKEN`.
- **R2 (the `CHUNKS` binding)** holds content-addressed chunk bytes under `chunks/ab/cd/<hex>`. When no binding exists — local dev, tests — chunk bytes stay inline in the `chunks` table instead. The rows are the existence oracle either way; nothing ever probes R2 for existence.
- **The edge cache** holds anonymous reads. Versioned URLs cache immutable for a year; pointers and the catalog cache for five minutes or an hour. The purge API (`CF_ZONE_ID` + `CF_PURGE_TOKEN`) keeps the mutable copies honest; without it the origin is authoritative and the TTL is the bound.

## Deploy

One-time setup in the Cloudflare dashboard:

1. Connect the repository, set the project root to `website`.
2. Build command: `npm run build:wasm && npm run build`. The `prebuild` step regenerates the docs payloads automatically.
3. Build output directory: `.svelte-kit/cloudflare`. Environment: Node 22.
4. Compatibility date: `2026-09-01` (the `wrangler.toml` in `website/` carries the local-dev flavor; the dashboard keeps the production date).
5. Add an R2 bucket and bind it as `CHUNKS`. Without the binding the registry runs — bytes stay inline in Turso — but that shape is for dev only: it bills Turso rows for every byte.
6. Set the environment variables below. Generate each secret with `openssl rand -hex 32`.
7. Optional: schedule the sweep below with any cron that can POST (a GitHub Actions schedule, a cron service). There is no in-worker cron handler; the weekly sweep is one authenticated POST.

Minimum viable deploy is steps 1–6 plus a first publish. The verification section at the end proves each piece.

## Environment

| Variable | Required | What it does |
|---|---|---|
| `TURSO_DATABASE_URL` | yes | libSQL HTTP endpoint, `libsql://…` |
| `TURSO_AUTH_TOKEN` | yes | token for that endpoint |
| `CHUNKS` (R2 binding) | prod yes | chunk bytes; absent means inline bytes (dev only) |
| `PUBLISH_TOKEN` | yes | the org publisher token (`rnxpub_…`). When unset, any `preview-…` string publishes — deliberate for local dev, a hole anywhere else. The server logs a warning on first use. |
| `ADMIN_TOKEN` | yes | takedowns, scope transfers, GC, advisory filing, download rollups, special delete (first approval) |
| `ADMIN_DELETE_TOKEN` | for special delete | second approval for `@std` hard removal; must differ from `ADMIN_TOKEN`. Unset means special delete answers 403. |
| `SITE_URL` | self-host yes | public origin (`https://registry.example.com`). Only its host is served in `GET /api/version`. Unset means the canonical production host. |
| `SESSION_PEPPER` | accounts yes | pepper for password hashing; the auth routes refuse to run without it unless `RNX_ALLOW_NO_PEPPER=1` (dev only) |
| `ADVISORY_SIGNING_KEY` | prod yes | 64-hex Ed25519 seed that signs the advisory feed. Unset means an ephemeral key per isolate — signatures verify within the isolate but not across deploys. |
| `QUOTA_R2_BYTES` | no | storage budget in bytes (default 10 GiB); the guard samples usage and refuses publishes past it |
| `QUOTA_GUARD` | no | set to `off` to disable the quota check while funding covers the bill |
| `REGISTRY_PLAN` | no | `paid` raises the publish caps (4 MiB source, 4096 files, 512 new chunks); anything else is the Free shape (16 KiB, 64 files, 32 new chunks) |
| `RESOLVE_MAX_SUBREQUESTS` | no | per-resolve store round-trip budget (default 50, the Free ceiling) |
| `CF_ZONE_ID`, `CF_PURGE_TOKEN` | prod yes | Cloudflare purge credentials. Unset means every mutation writes a `purge-skipped` audit row and the edge TTL bounds staleness. |
| `COMPATIBILITY_DATE` | dashboard | `2026-09-01`; keep the dashboard value in sync with `wrangler.toml` |

Never log these values. The server never does: the audit trail keeps hashes and coordinates, never tokens or bytes.

## A fresh database

A new deployment starts with an empty database — no dump, no seed step. The first query runs the migration ladder (`BASE_SCHEMA` plus additive `ALTER TABLE` backfills), then stamps `schema_version`. From there:

1. `POST /api/version` answers the spec and capability list.
2. Publish the standard library: `website/scripts/publish-std.mjs` batch-publishes `@std/*` with the org token.
3. First publish of a name claims its owner; later publishes need the same scope or the org token.

The ladder is covered by `website/tests/schema-from-zero.test.ts`, which starts from zero tables, migrates, and runs a publish-to-fetch round trip (publish, metadata read, chunk-bytes read, resolve) against the fresh stack.

## Publishing

`POST /api/v0/packages` with `X-RNX-Protocol: 0` and `Authorization: Bearer <token>`. The body is one JSON source-tree frame: `name`, `version`, `files` (`Project.config` required), `description`, `readme`, `docs`, `guides`, `engineRange`, `keywords`, `tags`, `license`, `requestId`, and `artifactDigest` when `Project.config` declares `artifacts`.

The server reads the package identity out of `Project.config` with its own parser, checks the frame against it, and rejects anything that disagrees — a lying client causes a rejection, never a stored lie. Then it canonicalizes (`.rnx` files minified, `Project.config` version redacted, everything else verbatim), chunks, runs the whole-tree capability analysis against the declared-permission ceiling, checks the storage quota, verifies the artifact index when one is declared, and writes one atomic batch. Uploads that fail validation cost nothing against quota; only stored versions count.

Idempotency: every mutation takes a client-generated `requestId`. Publish reads it from the frame (`requestId` field wins; the `X-RNX-Request-ID` header is the fallback). Yank, takedown, and unpublish accept it as a JSON body field or the same header. Repeating a mutation with the same key answers success with `duplicate: true` instead of a second write. `rnx publish` mints one key per invocation and reuses it across its own retries; `rnx unpublish` does the same.

`@std` is reserved: session publishes naming `@std/*` fail with 403 even when the scope check would pass. Only the org token seeds the standard library.

## Registry-key publish rule

Top-level `registry`/`registries` keys in the frame `Project.config` fail the publish with `E108` naming the keys: stored packages must not pin registry choice, the installer chooses. The success body still carries the verdict under `reportOnly` (empty on every 201 now that the gate rejects first), and the same predicate drives the admin scan below, so the count reads zero by construction.

Missing or unparseable config text counts nothing — the config gate upstream already rejects what it cannot parse.

Runtime access is enforced, not counted. A frame that names a runtime internal outside registry-verified `@std/*` — bare (`__rnx_foo()` with no declaration, an `E303` pointing at the declared form) or declared (`import { fn name(params): Ret } from runtime "mod"`, an `E108` naming the owning `@std/*` wrapper) — fails whole-tree analysis, so the publish never reaches 201. Only a `@std/*`-named frame (which the route above gates to the org token) may carry `from runtime` imports, and only for bytes that arrived through registry-verified `@std` provision; a `@std/`-prefixed file smuggled into any other frame is still rejected. Declared `from native` aliases are unaffected, and `__ffi_*` names never counted.

`GET /api/admin/publish-rules` (admin token) replays the registry-key predicate over the live set — one full whole-tree re-analysis per package, sequential — and returns the count plus the violating package names. It is read-only and bounded: `?limit=` caps the packages per call (default 200, max 1000), each package caps at 8 MiB of reassembled source, and anything skipped lands in `skipped` with a reason instead of failing the call. Whole-tree analysis costs about 0.25 ms per KiB of canonical source; store reads run as one batched version snapshot and one batched chunk read per scan (plus one cached dep load per `@std` top), so the store round trips stay flat as the package count grows instead of dying at the platform subrequest ceiling partway through. Page through with repeated calls on a large registry. `POST /api/admin/publish-rules` (admin token) files the `critical` advisory for every counted violator that lacks one — cause plus remedy in the detail — and answers which ids it filed versus which were already present. Filing is idempotent: repeated calls converge instead of duplicating, and the GET never files, so the scan stays side-effect free.

## Mutations

All of them purge the affected edge URLs, and all of them report success only after the purge verifies (when purge credentials are configured). Without credentials the mutation commits, the audit row says `purge-skipped`, and the origin stays authoritative.

- **Yank** (`POST …/@{version}/yank`, owner or org token inside 72h of publish; admin token with `{"cause"}` after): flips `live` to `yanked`. The owner call shares the unpublish clock below — one 72-hour window from the publish timestamp, not two. Past the window it answers `409 yank-window-closed`, naming the version age, the window, and the admin path. The admin call needs a recorded security cause (`vulnerability | cve | malicious | legal`), stored on the version status and in the audit entry; without one it answers `400 invalid-yank-cause`. Metadata keeps serving with the new status; pinned requirements still resolve, floating ranges move past. Yanking a non-live version is 409.
- **Takedown** (`POST …/@{version}/takedown`, admin token, `{"reason"}`): writes the tombstone row and flips the status in one transaction. Every later read answers `410 withdrawn` forever.
- **Unpublish** (`POST …/@{version}/unpublish`, owner or org token): deletes the version row inside the 72-hour window after publish. The number stays burned — the audit log keeps the publish entry, so a re-mint answers 409. Past the window the endpoint answers `409 unpublish-window-closed`; late removal goes through the admin yank or takedown above instead. `rnx unpublish <name@version>` calls this endpoint.
- **Scope transfer** (`POST /api/scopes/{scope}/transfer`, admin token, `{"newOwner", "force"}`): moves every version row to the new scope. It requires zero live versions unless `force` is true. The old names never re-mint: the transfer audit entries keep them burned.
- **Special delete** (`DELETE /api/scopes/std/{name}@{version}`, admin token plus `X-RNX-Second-Approval`): exists for `@std/*` only, decided server-side. The row is hard-deleted, the audit entry survives, and later reads answer the `410 withdrawn (std-purge)` variant. There is no CLI path.

Numbers never come back: the reuse check reads live rows, tombstones, and the audit log together.

## Garbage collection

`POST /api/admin/gc` (admin token) runs the three-phase sweep: mark orphan chunks past the 30-day grace window, re-verify each batch against the live version rows (a chunk claimed mid-sweep is skipped, never deleted), then delete with one audit row per sweep. `?dry_run=1` (or `{"dry_run": true}`) lists what would be deleted and touches nothing. The same call prunes operational audit noise (`purge-skipped`, `purge-failed`, `gc-sweep`) older than one year. Version history — publish, unpublish, takedown, transfer, special-delete — is never pruned: the reuse check depends on it.

Deletes live on this path only. No other code path deletes chunk bytes.

## Advisories

`POST /api/v0/advisories` (admin token, protocol header) files an advisory; `POST /api/v0/advisories/<id>/withdraw` withdraws it. `GET /api/v0/advisories` serves the active set plus the operator's Ed25519 signature over the canonical byte form. Clients pin the key with `RNX_ADVISORY_KEY` and fail closed on a bad signature. `rnx audit` matches the locked deplock — versions and artifact pins — against the feed: `critical` fails the operation, everything else warns.

## Verification

`npm run check:registry -- <origin>` probes a deployment: discovery shape, spec header, capability list, one anonymous resolve, and the immutable cache headers. It performs no mutations and needs no token.

```sh
npm run check:registry -- https://registry.example.com
```

Manual pass after every deploy change:

1. `GET /api/version` → `{"spec": 0, "capabilities": […], "registry": "<your host>"}` plus the `rnx-registry-spec: 0` header.
2. Publish a scratch package with the org token, fetch its metadata, chunk manifest, and one chunk; verify the chunk hash locally.
3. `POST /api/v0/resolve` with the scratch package pinned and ranged; confirm the max-satisfying pick and the `yanked` flag behavior.
4. Yank the scratch version, confirm the metadata status flips; takedown a second scratch version, confirm 410; unpublish a third inside the window, confirm 404 and a burned number.
5. `POST /api/admin/gc` with `dry_run=1`; confirm it lists orphans without deleting.
6. Never run these steps against the production registry. All conformance tests in this repo run against throwaway databases (`website/tests/libsql-fake.ts`); production mutations have no test harness and get none.

## Known gaps

Each gap not closed by this change has an owner and a reason:

- **Self-serve unpublish re-mint semantics** (spec §24): unpublish burns the number today. Whether a republish inside the window should ever re-open it needs an owner ruling plus a CLI surface; until then the number stays burned. Carried forward explicitly: the owner-yank gate added alongside shares the same window and the same burned-number rule, and does not itself re-open anything. Owner: registry maintainer.
- **Funding and operator broadcast notices** (spec §23 beyond advisories): the signed advisory feed covers the security path, which is what `rnx audit` consumes. Package funding links and operator broadcasts have no transport and no client consumer. Owner: registry maintainer.
- **Download-count accuracy** (spec §10): counts read zero until the operator wires a counter source into the rollup endpoint. Approximate by design; never correctness. Owner: deploying operator.
- **Quarterly R2 reconciliation** (spec §§8–9): the sampling of the R2 listing against the version-row manifests is a manual operational task. Owner: deploying operator.
- **Unconfigured purge** (spec §§2, 6): without `CF_ZONE_ID`/`CF_PURGE_TOKEN` the one-year immutable TTL bounds the stale window for yanked/taken-down metadata. Tooling that needs the current status reads `?origin-direct=1`, which is `no-store`. Owner: deploying operator.
- **Free-plan first-publish ceiling** (spec §§8c, 24b): whole-tree analysis of more than ~16 KiB of source exceeds the 10 ms CPU budget. Publish large trees from the Paid plan; small-churn incrementals fit Free. Owner: platform constant, documented not fixed.
