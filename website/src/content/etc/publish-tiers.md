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

Only accepted versions count against quota — rejected uploads, failed validations, and idempotent retries cost nothing. What counts is the stored bytes of live versions, sampled against the storage budget with a pity-style ramp: a flat base sample rate while usage is comfortable, rising toward certain measurement as the budget fills. The per-version 512KB cap and a weekly per-package publish cap of 20 keep any single publisher from moving the needle far.

TODO(owner): publish the exact numeric rate limits and quota thresholds so clients can display them instead of discovering them.

## Abuse backoff is separate

Velocity and burst protection runs on its own track from storage quota. A publisher tripping the abuse tripwires ([Abuse](/etc/abuse)) gets slowed with 429s and backoff while the storage budget keeps measuring only accepted bytes — burning quota is never the punishment, and serving a backoff never spends budget. The two counters reset independently.

## Paid tiers

The free tier covers public packages within the caps above. Paid tiers add four things, each aimed at teams rather than individuals:

- **Private scopes**, invisible to anyone outside the owning org.
- **Higher caps** on version size and publish rate.
- **Provenance**, recording which identity and pipeline cut each version.
- **SPDX filtering**, resolving and auditing dependencies by license expression.

TODO(owner): paid tiers are not offered yet — confirm availability, pricing, and which of the four lands first before linking this page from anywhere billing-adjacent.

## Large-blob hosting (planned)

Source packages stay small by rule, but platform binaries, datasets, and media need a home with different economics: an LFS-style blob addon with its own meter (indicative: 1 GB for $1, reads free under standard rate limits). Blobs referenced by live package versions reject deletion naming the referencing versions — reproducibility outranks tidiness. Unreferenced blobs delete freely; anything else needs the deprecate-first path or an audited force override. Closed-source libraries ride the same rails under team plans (private scopes plus storage and limits). None of this exists yet; the source registry above is unaffected either way.
