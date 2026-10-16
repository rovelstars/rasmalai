---
title: "Versioning and Yank"
description: "Strict semver, prerelease rules, dist-tags, yank semantics, and the unpublish window."
section: "Registry"
order: "4"
icon: "Recycle"
---

# Versioning and Yank

Versions are strict semver — `X.Y.Z` with an optional prerelease tail — and every `name@version` is immutable. Publishing an already-used number fails with 409, and no later event re-opens it: not a yank, not a takedown, not a scope transfer. The server checks live rows, tombstones, and the audit log together, so even a deleted version keeps its number burned.

## Ranges

A requirement is `*`, `latest`, an exact `X.Y.Z`, `^X.Y.Z`, `~X.Y.Z`, or `>=X.Y.Z`. Anything else is a 400 `bad-range`. The semantics:

- `*` and `latest` match any stable version, never a prerelease.
- An exact version matches exactly it. A prerelease matches only when the range itself names a prerelease with the same major, minor, and patch.
- `^1.2.3` means at least `1.2.3` within the same major. The `0.x` lines hold tighter: `^0.2.3` pins the minor as well, and `^0.0.3` pins the patch — exactly the npm rule, so early packages do not float across breaking changes by accident.
- `~1.2.3` means at least `1.2.3` within the same major and minor.
- `>=1.2.3` means that version or newer.

The resolver picks the highest satisfying version, walks manifests transitively, and fails the whole resolve on conflicting ranges (`unsatisfiable`) or dependency cycles (`cycle`).

## Prereleases never auto-resolve

A prerelease only resolves when someone asks for it by its full name. Floating ranges skip prereleases entirely, and `latest` is computed as the highest stable version — publishing `1.3.0-beta.1` never moves the `latest` pointer. Named channels arrive with dist-tags: `latest` will auto-advance on stable publishes but never on prereleases, `rnx publish --tag beta` will move one tag without touching `latest`, and tag names will work in requirement position. Until servers advertise the `dist-tags` capability, there are no named tags and the computed-`latest` behavior above is the whole story.

## Yank warns forever

Yanking a live version flips its status without removing anything. The metadata still serves, now marked `yanked`; pinned requirements keep resolving to it, but every such build prints a warning naming the version and its suggested replacement; floating ranges and the `latest` pointer move past it. Yanking a version that is not live fails — and like everything else here, a yanked number can never be re-published.

## The 72-hour unpublish window

Mistakes in the first 72 hours after publish — a wrong tarball, a leaked secret, a version cut from the wrong branch — can be self-serve unpublished by the owner. Past that window the version is permanent history: yank it, publish a fixed successor, and let pins age out. Unpublish burns the number either way.

Within the window, the owner removes the version with `rnx unpublish <name>@<version>`; the command disappears the version, burns the number, and leaves an audit entry.

## Staff-only hard removal

Anything beyond the owner's window needs staff. Takedown writes a tombstone row and flips the version in one transaction; every later read answers 410 with the reason, cached immutably. The `@std/*` scope has one further carve-out — a hard delete needing the org admin plus a second approval, with no CLI path — and even there the audit entry survives. Removal is always recorded; silent deletion does not exist.

## Pinned builds always resolve

A build pinned by `Project.deplock` resolves from the local cache without touching the network, and the recorded integrity digest must still match — bytes that differ from the lockfile abort the build instead of installing. Yanked pins keep building with their warning. Tombstoned pins fail with the 410, the reason, and the upgrade pointer, because a removal the registry stands behind must not be papered over by a cache.

## Notices come from the registry, never from packages

Warnings, funding appeals, and critical holds render from registry-signed payloads only — no code path renders package content as a notice, so spoofing is structurally impossible. Funding banners are maintainer opt-in with a set link, shown at most once per resolution; yank, quota, and grace notices show on the operations they concern. Only the critical tier (tombstone, security hold) fails operations; everything else informs without blocking.
