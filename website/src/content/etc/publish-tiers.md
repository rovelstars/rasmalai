---
title: "Publish Tiers and Quotas"
description: "What publishing costs, what counts against quota, and what paid tiers add."
section: "Registry"
order: "5"
icon: "KeyRound"
---

# Publish Tiers and Quotas

Reads are anonymous and edge-cached. Writes are authenticated, budgeted, and metered — the registry stores bytes on other people's behalf, and the cost guard sits in front of every upload.

## Anonymous publish never happens

Publishing needs a token: pass `--token` or set `RNX_TOKEN`, or the CLI stops with `E501` before sending anything. The server matches the bearer token exactly and checks scope membership for scoped packages, so a valid token for someone else's scope still fails. There is no unauthenticated write path and no plan for one.

## Quotas count accepts, not attempts

Only accepted versions count against quota — rejected uploads, failed validations, and idempotent retries cost nothing. What counts is the canonical bytes of live versions (`.rnx` minified, the form the server stores), sampled against the storage budget with a pity-style ramp: a flat base sample rate while usage is comfortable, rising toward certain measurement as the budget fills. The plan-shaped source caps (16 KiB Free, 4 MiB Paid) and a weekly per-package publish cap of 20 keep any single publisher from moving the needle far. Republishing an unchanged tree stores nothing new: identical canonical bytes reuse every stored object and skip re-analysis.

Publishing is metered by what you store: each accepted version counts against your quota, and rejected or retried uploads never do.

## Abuse backoff is separate

Velocity and burst protection runs on its own track from storage quota. A publisher tripping the abuse tripwires ([Abuse](/etc/abuse)) gets slowed with 429s and backoff while the storage budget keeps measuring only accepted bytes — burning quota is never the punishment, and serving a backoff never spends budget. The two counters reset independently.

## Paid tiers

The free tier covers public packages within the caps above. Paid tiers add four things, each aimed at teams rather than individuals:

- **Private scopes**, invisible to anyone outside the owning org.
- **Higher caps** on source size (4 MiB), file and chunk counts, and publish rate.
- **Provenance**, recording which identity and pipeline cut each version.
- **SPDX filtering**, resolving and auditing dependencies by license expression.

Paid tiers are not on sale yet. Everything above applies unchanged to every tier; a paid tier raises caps and adds private scopes, provenance, and license filtering, never paywalled correctness.

## Artifact bytes live on origins (pointer-mode)

Platform binaries, datasets, and media are OCI artifacts, never registry bytes: the registry stores the pin and the parsed descriptor list as version metadata and nothing else. Origins bear the byte costs while the registry owns pins and policy — there is no hosted blob store to meter per publisher and no hosted bytes to delete. A publish with artifacts moves kilobytes (one capped index fetch) and stores metadata rows only. An origin that deletes or moves pinned bytes fails loud: the hash-named fetch 404s naming the dead pin, and recovery is a new version with a new pin. A future operator mirror would be an availability optimization only — identical hashes, no wire-semantic change — and its economics, if it ever lands, will be specified then. Until that edit, no blob-metering or blob-deletion machinery exists.
