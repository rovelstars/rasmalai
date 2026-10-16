---
title: "Abuse and Takedowns"
description: "What trips review, what gets tombstoned, and how to report a package."
section: "Registry"
order: "3"
icon: "ShieldAlert"
---

# Abuse and Takedowns

Most enforcement is automatic. The permission scanner rejects publishes whose code reaches further than the manifest declares, with a 400 naming the symbol, the file, and the missing grant. What follows covers the rest: the patterns that send a package to human review, and what happens there.

## Tripwires

Three patterns flag a package or publisher for review:

- **Typosquatting.** A new name close enough to an established package to catch typos — transposed letters, a missing hyphen, a swapped scope — holds for review instead of going live on trust.
- **Velocity.** A sudden burst of publishes, versions, or scopes from one publisher, far above the weekly per-package rhythm honest work produces, pauses the account's uploads pending a look.
- **Permissions.** A new version that widens its capability grants over the previous one — especially toward network, process, or `unsafe` — gets a second look even when the manifest declares everything correctly.

A flag is not a verdict. Flagged uploads wait for a human; clean ones proceed, and the review aims at hours, not weeks.

## What gets tombstoned

Yank handles the ordinary end of life (see [Deprecation](/etc/deprecation)). Tombstones are for packages that must stop resolving: malware and harvesters, license violations where the grant was never valid, and confirmed typosquats published to deceive. A tombstoned version answers 410 `{ code: "withdrawn", reason }` forever, the number stays burned, and the audit entry survives so the removal itself is on record. Nothing — not the original owner, not a scope transfer — can re-mint it.

## How to report

If a package harms machines, steals credentials, squats a name, or ships code its license forbids, report it with the package name, the exact version, and what you observed. Include the audit id if you already hit a 410, and the resolve `requestId` if the failure came from the resolver.

Reports go to the Discord server linked in the site footer. Flagged uploads are reviewed in a weekly digest; a report normally gets a human answer within a week, sooner if it names a security issue.
