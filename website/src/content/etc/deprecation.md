---
title: "Deprecation"
description: "How versions die with dignity: deprecate, yank, tombstone."
section: "Registry"
order: "6"
icon: "TriangleAlert"
---

# Deprecation

Versions die in three steps, each louder than the last: deprecate, yank, tombstone. Every step keeps the number burned — nothing on this ladder ever frees a version for reuse.

## 1. Deprecate: the advisory

Deprecation marks a version or package as discouraged while it still resolves cleanly. Pins keep building with no new warnings, floating ranges keep selecting it, and the only change is the signal: prefer the successor. Deprecate when a better version exists but the old one still works — the rename, the re-scope, the API you wish you had designed first.

TODO(owner): settle the exact deprecate mechanism — a metadata flag the registry serves versus a README-level badge — before documenting the command.

## 2. Yank: the warning

Yank is for versions with something wrong with them that does not endanger anyone: the flawed migration, the accidentally breaking minor, the release cut from the wrong branch after the unpublish window closed. Yanked metadata serves with its status set, pinned builds resolve with a warning naming the replacement, and floating ranges move on. Yank when the old version should keep working for whoever pinned it but nobody new should arrive.

## 3. Tombstone: the removal

Tombstones are for versions that must stop resolving — malware, credential theft, license violations, confirmed deception (see [Abuse](/etc/abuse)). Staff write the tombstone; every later read answers 410 with the reason and an audit id, forever. Tombstone when serving the bytes one more time is itself the harm.

## Walk the ladder in order

The dignified sequence for a flawed release: publish the fixed successor first, deprecate or yank the flawed version with the replacement named, and reserve tombstones for danger. Never yank without a suggested replacement — a warning that points nowhere strands every pinned build it touches. And never reach for staff removal to fix an ordinary mistake inside your own 72-hour window ([Versioning and Yank](/etc/versioning-and-yank)); that path exists, use it, and leave tombstones for what they are for.
