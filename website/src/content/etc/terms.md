---
title: "Registry Terms"
description: "Acceptable use, publisher responsibility, and the registry's removal rights."
section: "Registry"
order: "2"
icon: "BookOpen"
---

# Registry Terms

This is the skeleton of the registry's terms of service — the rules both sides agree to when you publish or install. It is deliberately short; the enforcement details live in the abuse, versioning, and tier pages it points at.

## Acceptable use

Publish code you have the right to distribute, and nothing that harms the people who install it. That rules out malware, credential harvesters, and obfuscated payloads, as well as content that is illegal to redistribute, spam or squatted names held for ransom, and uploads designed to waste shared resources. The upload rules ([Registry Upload Rules](/etc/registry-rules)) already narrow what an archive can hold; these terms narrow what it may be.

## Publishers own their content

You are responsible for what you publish: its correctness, its license grant, and its behavior on other people's machines. The registry stores and serves your bytes unchanged — it does not review code for quality, and hosting a package is not an endorsement of it. Keep the `LICENSE` file accurate and keep your contact reachable, because abuse reports about your packages come to you first.

## The registry may remove

The operator may yank or take down any version that breaks these terms, following the ladder in [Deprecation](/etc/deprecation): yank for the flawed-but-harmless, tombstone for the dangerous. Tombstoned versions answer 410 forever with the reason and an audit id; the version number stays burned so nothing can silently take its place. Enforcement mistakes can be appealed through the same channel abuse is reported on ([Abuse](/etc/abuse)).

## No warranty

The registry is offered as-is. Version metadata, uptime, and download counts carry no guarantees — pin your builds with `Project.deplock` rather than trusting any of them.

TODO(owner): fill in governing law, operator contact, effective date, and the appeal turnaround before these terms go live.
