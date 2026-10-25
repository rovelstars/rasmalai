import { sha256Hex } from './chunks.js';

export const OCI_INDEX_CAP = 1024 * 1024;

export const ARTIFACT_DIGEST_RE = /^sha256:[0-9a-f]{64}$/;

const REF_SEGMENT = '[a-z0-9]+(?:[._-][a-z0-9]+)*';
const BARE_REF_RE = new RegExp(
	`^${REF_SEGMENT}(?:\\.${REF_SEGMENT})*(?::[0-9]+)?(?:/${REF_SEGMENT})+$`
);

const TARGET_PART = '[a-z0-9_]+';
const TARGET_RE = new RegExp(`^${TARGET_PART}-${TARGET_PART}-${TARGET_PART}(?:-${TARGET_PART})?$`);
const GENERATOR_RE = /^[A-Za-z0-9._-]+ [0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9._-]+)?$/;
const UPSTREAM_RE = /^[A-Za-z0-9._-]+@sha256:[0-9a-f]{64}$/;

const KIND_TO_ROLES: Record<string, string[]> = {
	binary: ['runtime', 'wasm'],
	bindings: ['bindgen'],
	media: ['media']
};

export function isArtifactDigest(value: string): boolean {
	return ARTIFACT_DIGEST_RE.test(value);
}

export function bareRefError(ref: string): string | null {
	if (typeof ref !== 'string' || ref === '' || !BARE_REF_RE.test(ref) || ref.includes('@')) {
		return 'field `artifacts` must be a bare OCI repo ref string (e.g. `artifacts: "ghcr.io/org/sqlite3"`); tags, digests and keyed tables are discovery hints or pins, never the declaration';
	}
	const last = ref.slice(ref.lastIndexOf('/') + 1);
	if (last.includes(':')) {
		return 'field `artifacts` must be a bare OCI repo ref string (e.g. `artifacts: "ghcr.io/org/sqlite3"`); an explicit tag is a discovery hint only, never stored';
	}
	return null;
}

export interface ArtifactRefParts {
	registry: string;
	repo: string;
	tag: string;
	bare: string;
}

export function splitArtifactRef(ref: string): ArtifactRefParts | null {
	if (bareRefError(ref) !== null) return null;
	const slash = ref.indexOf('/');
	const registry = ref.slice(0, slash);
	const repo = ref.slice(slash + 1);
	return { registry, repo, tag: 'latest', bare: ref };
}

export function artifactIndexUrl(ref: string): { url: string; parts: ArtifactRefParts } | null {
	const parts = splitArtifactRef(ref);
	if (!parts) return null;
	return {
		url: `https://${parts.registry}/v2/${parts.repo}/manifests/${parts.tag}`,
		parts
	};
}

export interface ArtifactEntry {
	digest: string;
	role: string;
	title: string;
	kind: string;
	target: string;
	generator: string;
	upstream: string;
	os: string | null;
	architecture: string | null;
}

export interface ArtifactPin {
	ref: string;
	digest: string;
	entries: ArtifactEntry[];
}

export type OciFailure =
	| { ok: false; cause: 'missing-assertion'; message: string }
	| { ok: false; cause: 'digest-mismatch'; message: string }
	| { ok: false; cause: 'unreachable-origin'; message: string }
	| { ok: false; cause: 'oversize-index'; message: string }
	| { ok: false; cause: 'bad-annotation'; message: string };

function fail<C extends OciFailure['cause']>(cause: C, message: string): Extract<OciFailure, { cause: C }> {
	return { ok: false, cause, message } as Extract<OciFailure, { cause: C }>;
}

export async function fetchArtifactIndex(
	ref: string,
	fetchImpl: typeof fetch = fetch
): Promise<{ ok: true; bytes: Uint8Array; digest: string } | OciFailure> {
	const target = artifactIndexUrl(ref);
	if (!target) {
		return fail(
			'bad-annotation',
			`E108: artifact ref \`${ref}\` is not a bare OCI repo ref (expected \`ghcr.io/org/name\`)`
		);
	}
	let res: Response;
	try {
		res = await fetchImpl(target.url, {
			headers: {
				Accept: 'application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json'
			},
			cache: 'no-store'
		});
	} catch {
		return fail('unreachable-origin', `E108: artifact origin unreachable (cannot reach \`${target.parts.registry}\`)`);
	}
	if (!res.ok) {
		return fail(
			'unreachable-origin',
			`E108: artifact origin unreachable (registry answered HTTP ${res.status} for \`${ref}\`)`
		);
	}
	const body = res.body;
	if (!body) {
		const buf = new Uint8Array(await res.arrayBuffer());
		if (buf.byteLength > OCI_INDEX_CAP) {
			return fail('oversize-index', `E108: artifact index exceeds the 1 MiB cap (ref \`${ref}\`)`);
		}
		const digest = `sha256:${await sha256Hex(buf)}`;
		return { ok: true, bytes: buf, digest };
	}
	const reader = body.getReader();
	const chunks: Uint8Array[] = [];
	let total = 0;
	for (;;) {
		const { done, value } = await reader.read();
		if (done) break;
		total += value.byteLength;
		if (total > OCI_INDEX_CAP) {
			await reader.cancel().catch(() => undefined);
			return fail('oversize-index', `E108: artifact index exceeds the 1 MiB cap (ref \`${ref}\`)`);
		}
		chunks.push(value);
	}
	const merged = new Uint8Array(total);
	let off = 0;
	for (const chunk of chunks) {
		merged.set(chunk, off);
		off += chunk.byteLength;
	}
	const digest = `sha256:${await sha256Hex(merged)}`;
	return { ok: true, bytes: merged, digest };
}

export function parseArtifactIndex(
	ref: string,
	bytes: Uint8Array,
	asserted: string
): { ok: true; pin: ArtifactPin } | OciFailure {
	let doc: unknown;
	try {
		doc = JSON.parse(new TextDecoder().decode(bytes));
	} catch {
		return fail('bad-annotation', `E108: artifact index is not valid JSON (ref \`${ref}\`)`);
	}
	if (!doc || typeof doc !== 'object' || Array.isArray(doc)) {
		return fail('bad-annotation', `E108: artifact index must be a JSON object (ref \`${ref}\`)`);
	}
	const manifests = (doc as Record<string, unknown>)['manifests'];
	if (!Array.isArray(manifests) || manifests.length === 0) {
		return fail('bad-annotation', `E108: artifact index carries no manifests (ref \`${ref}\`)`);
	}
	const entries: ArtifactEntry[] = [];
	for (const item of manifests) {
		const entry = checkDescriptor(ref, item);
		if (!entry.ok) return entry;
		entries.push(entry.entry);
	}
	return { ok: true, pin: { ref, digest: asserted, entries } };
}

function checkDescriptor(
	ref: string,
	item: unknown
): { ok: true; entry: ArtifactEntry } | OciFailure {
	if (!item || typeof item !== 'object' || Array.isArray(item)) {
		return fail('bad-annotation', `E108: artifact descriptor must be an object (ref \`${ref}\`)`);
	}
	const row = item as Record<string, unknown>;
	const digest = row['digest'];
	if (typeof digest !== 'string' || !ARTIFACT_DIGEST_RE.test(digest)) {
		return fail(
			'bad-annotation',
			`E108: artifact descriptor carries a bad digest \`${String(digest).slice(0, 24)}\` (ref \`${ref}\`; expected \`sha256:<64 hex chars>\`)`
		);
	}
	const annotations = row['annotations'];
	if (!annotations || typeof annotations !== 'object' || Array.isArray(annotations)) {
		return fail('bad-annotation', `E108: artifact descriptor ${digest} carries no annotations (ref \`${ref}\`)`);
	}
	const ann = annotations as Record<string, unknown>;
	const str = (key: string): string | null =>
		typeof ann[key] === 'string' ? (ann[key] as string) : null;
	const kind = str('run.rasmalai.kind');
	const role = str('run.rasmalai.role');
	const target = str('run.rasmalai.target');
	const generator = str('run.rasmalai.generator');
	const upstream = str('run.rasmalai.upstream');
	const title = str('org.opencontainers.image.title');
	if (kind !== 'binary' && kind !== 'bindings' && kind !== 'media') {
		return fail(
			'bad-annotation',
			`E108: artifact descriptor ${digest} carries a bad run.rasmalai.kind \`${kind ?? 'missing'}\` (ref \`${ref}\`; expected \`binary\`, \`bindings\` or \`media\`)`
		);
	}
	if (!role || !(KIND_TO_ROLES[kind] as string[]).includes(role)) {
		return fail(
			'bad-annotation',
			`E108: artifact descriptor ${digest} carries a bad run.rasmalai.role \`${role ?? 'missing'}\` (ref \`${ref}\`; kind \`${kind}\` admits roles ${(KIND_TO_ROLES[kind] as string[]).join(', ')})`
		);
	}
	if (!target || !TARGET_RE.test(target)) {
		return fail(
			'bad-annotation',
			`E108: artifact descriptor ${digest} carries a bad run.rasmalai.target \`${target ?? 'missing'}\` (ref \`${ref}\`; expected an arch-vendor-os-env triple)`
		);
	}
	if (!generator || !GENERATOR_RE.test(generator)) {
		return fail(
			'bad-annotation',
			`E108: artifact descriptor ${digest} carries a bad run.rasmalai.generator \`${generator ?? 'missing'}\` (ref \`${ref}\`; expected \`<name> <semver>\`)`
		);
	}
	if (!upstream || !UPSTREAM_RE.test(upstream)) {
		return fail(
			'bad-annotation',
			`E108: artifact descriptor ${digest} carries a bad run.rasmalai.upstream \`${upstream ?? 'missing'}\` (ref \`${ref}\`; expected \`<name>@sha256:<hex>\`)`
		);
	}
	if (!title || title.includes('/') || title.includes('\\') || title === '.' || title === '..') {
		return fail(
			'bad-annotation',
			`E108: artifact descriptor ${digest} carries a bad org.opencontainers.image.title \`${title ?? 'missing'}\` (ref \`${ref}\`; expected a plain filename)`
		);
	}
	let os: string | null = null;
	let architecture: string | null = null;
	const platform = row['platform'];
	if (platform !== undefined && platform !== null) {
		if (!platform || typeof platform !== 'object' || Array.isArray(platform)) {
			return fail(
				'bad-annotation',
				`E108: artifact descriptor ${digest} carries a bad platform (ref \`${ref}\`; expected \`os\` and \`architecture\`)`
			);
		}
		const plat = platform as Record<string, unknown>;
		if (typeof plat['os'] !== 'string' || (plat['os'] as string) === '') {
			return fail(
				'bad-annotation',
				`E108: artifact descriptor ${digest} carries a bad platform.os (ref \`${ref}\`; expected a non-empty string)`
			);
		}
		if (typeof plat['architecture'] !== 'string' || (plat['architecture'] as string) === '') {
			return fail(
				'bad-annotation',
				`E108: artifact descriptor ${digest} carries a bad platform.architecture (ref \`${ref}\`; expected a non-empty string)`
			);
		}
		os = plat['os'] as string;
		architecture = plat['architecture'] as string;
	}
	return { ok: true, entry: { digest, role, title, kind, target, generator, upstream, os, architecture } };
}

export function checkArtifactAgreement(
	declared: string | null,
	asserted: string | null
): { ok: true } | OciFailure {
	if (declared && !asserted) {
		return fail(
			'missing-assertion',
			`E108: package declares artifacts \`${declared}\` but the publish frame carries no artifactDigest (missing assertion; pass \`--artifact sha256:<hex>\`)`
		);
	}
	if (!declared && asserted) {
		return fail(
			'missing-assertion',
			'E108: publish frame carries artifactDigest but Project.config declares no `artifacts` (declaration and assertion must agree)'
		);
	}
	return { ok: true };
}
