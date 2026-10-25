import {
	publishCanonicalVersion,
	type CanonicalPublishPayload as CanonicalPayload
} from '../src/lib/server/db.js';

export type { CanonicalPayload as CanonicalPublishPayload };

// Seeds one version row through the canonical publish path. The digest
// defaults to the readable `sha-<full>-<version>` stand-in the resolve
// tests assert on; publish-path tests override it with a real digest.
export function seedPayload(
	name: string,
	version: string,
	overrides: Partial<CanonicalPayload> = {}
): CanonicalPayload {
	const digest = `sha-${name}-${version}`;
	return {
		name,
		version,
		description: 'fixture',
		author: 'test',
		license: 'MIT',
		tags: [],
		readme: '',
		docJson: '{"modules":[]}',
		checksum: digest,
		manifestJson: '{}',
		requestId: '',
		entries: [],
		manifestDigest: digest,
		verdictKey: '',
		artifactDigest: null,
		artifactMeta: null,
		units: [],
		facts: [],
		edges: { edges: [], permsDigest: '', files: [] },
		...overrides
	};
}

export function seedPackage(
	env: Record<string, string | undefined>,
	name: string | CanonicalPayload,
	version?: string,
	overrides: Partial<CanonicalPayload> = {}
) {
	if (typeof name === 'object') {
		return publishCanonicalVersion(env, { ...seedPayload(name.name, name.version), ...name });
	}
	return publishCanonicalVersion(env, seedPayload(name, version ?? '1.0.0', overrides));
}
