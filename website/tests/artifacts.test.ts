import { describe, it, before, after, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { POST as publishPOST } from '../src/routes/api/v0/packages/+server.ts';
import { GET as readGET } from '../src/routes/api/v0/packages/[...path]/+server.ts';
import { POST as resolvePOST } from '../src/routes/api/v0/resolve/+server.ts';
import { clearVersionIndex, getVersionIndexRow } from '../src/lib/server/db.js';
import { isArtifactDigest } from '../src/lib/server/oci.js';

let fake: FakeLibsql;

const REF = 'oci.test/org/sqlite3';

function config(name: string, version: string, artifacts: string | null): string {
	const tail = artifacts === null ? '' : `,\n    artifacts: "${artifacts}"`;
	return `export default {\n    project: {\n        name: "${name}",\n        version: "${version}"\n    },\n    entries: { main: "src/main.rnx" }${tail}\n}\n`;
}

function frame(
	name: string,
	version: string,
	artifacts: string | null,
	artifactDigest: string | null
): Record<string, unknown> {
	return {
		name,
		version,
		files: [
			{ path: 'Project.config', content: config(name, version, artifacts) },
			{ path: 'LICENSE', content: 'MIT License\n' },
			{ path: 'src/main.rnx', content: 'export fn main(): Void {\n  let x = 1;\n}\n' }
		],
		description: 'fixture',
		readme: `# ${name}\n`,
		license: 'MIT',
		artifactDigest
	};
}

function annotations(role: string, kind: string, title: string): Record<string, string> {
	return {
		'run.rasmalai.kind': kind,
		'run.rasmalai.role': role,
		'run.rasmalai.target': 'x86_64-unknown-linux-gnu',
		'run.rasmalai.generator': 'rnx-bindgen 0.1.0',
		'run.rasmalai.upstream': `sqlite@sha256:${'cd'.repeat(32)}`,
		'org.opencontainers.image.title': title
	};
}

function indexBytes(manifests: Array<Record<string, unknown>>): Uint8Array {
	return new TextEncoder().encode(JSON.stringify({ schemaVersion: 2, manifests }));
}

function descriptor(digest: string, role: string, kind: string, title: string, platform?: { os: string; architecture: string }): Record<string, unknown> {
	const row: Record<string, unknown> = { digest, annotations: annotations(role, kind, title) };
	if (platform) row['platform'] = platform;
	return row;
}

function indexDigest(bytes: Uint8Array): string {
	return `sha256:${createHash('sha256').update(bytes).digest('hex')}`;
}

let origin: Map<string, Uint8Array | null> = new Map();
let unreachable: string[] = [];
const realFetch = globalThis.fetch;

function serveOrigin(host: string, bytes: Uint8Array | null): string {
	origin.set(`https://${host}/v2/org/sqlite3/manifests/latest`, bytes);
	return indexDigest(bytes ?? new Uint8Array());
}

function stubFetch() {
	globalThis.fetch = (async (input: unknown) => {
		const url = String(input);
		const host = (() => {
			try {
				return new URL(url).host;
			} catch {
				return '';
			}
		})();
		if (unreachable.includes(host)) throw new Error('connection refused');
		if (!url.includes('/v2/')) return realFetch(input as never);
		if (!origin.has(url)) {
			return new Response('{"code":"not-found"}', { status: 404 });
		}
		const bytes = origin.get(url);
		if (bytes === null || bytes === undefined) {
			return new Response('oops', { status: 500 });
		}
		return new Response(bytes as BodyInit, { status: 200 });
	}) as typeof fetch;
}

function publish(name: string, version: string, body: Record<string, unknown>): Promise<Response> {
	const request = new Request('https://registry.test/api/v0/packages', {
		method: 'POST',
		headers: {
			'X-RNX-Protocol': '0',
			authorization: 'Bearer preview-token-123',
			'Content-Type': 'application/json'
		},
		body: JSON.stringify(body)
	});
	return publishPOST({
		request,
		url: new URL('https://registry.test/api/v0/packages'),
		platform: { env: fake.env }
	} as never);
}

function countVersions(full: string): number {
	const [scope, name] = full.startsWith('@') ? [full.slice(1).split('/')[0], full.slice(1).split('/')[1]] : ['', full];
	const row = fake.db
		.prepare(
			`SELECT count(*) AS n FROM package_versions v JOIN packages p ON p.id = v.package_id
			 WHERE p.scope = ? AND p.name = ?`
		)
		.get(scope, name) as { n: number };
	return Number(row.n);
}

before(async () => {
	fake = await startFakeLibsql();
	stubFetch();
});

after(async () => {
	globalThis.fetch = realFetch;
	await fake.close();
});

beforeEach(() => {
	clearVersionIndex();
	fake.reset();
	origin = new Map();
	unreachable = [];
});

describe('artifact declaration and assertion agreement', () => {
	it('rejects a declared artifact with no frame digest', async () => {
		const res = await publish('@acme/declared', '1.0.0', frame('@acme/declared', '1.0.0', REF, null));
		assert.equal(res.status, 400);
		const body = (await res.json()) as Record<string, unknown>;
		assert.match(String(body['error']), /missing assertion/);
		assert.equal(countVersions('@acme/declared'), 0);
	});

	it('rejects a frame digest with no declared artifacts', async () => {
		const res = await publish(
			'@acme/undeclared',
			'1.0.0',
			frame('@acme/undeclared', '1.0.0', null, `sha256:${'aa'.repeat(32)}`)
		);
		assert.equal(res.status, 400);
		const body = (await res.json()) as Record<string, unknown>;
		assert.match(String(body['error']), /must agree/);
		assert.equal(countVersions('@acme/undeclared'), 0);
	});

	it('rejects a malformed frame digest', async () => {
		const res = await publish(
			'@acme/malformed',
			'1.0.0',
			frame('@acme/malformed', '1.0.0', REF, 'sha256:XYZ')
		);
		assert.equal(res.status, 400);
		const body = (await res.json()) as Record<string, unknown>;
		assert.match(String(body['error']), /artifactDigest/);
		assert.equal(countVersions('@acme/malformed'), 0);
	});
});

describe('artifact origin verification', () => {
	it('rejects an unreachable origin fail-closed', async () => {
		unreachable = ['gone.test'];
		const res = await publish(
			'@acme/gone',
			'1.0.0',
			frame('@acme/gone', '1.0.0', 'gone.test/org/sqlite3', `sha256:${'aa'.repeat(32)}`)
		);
		assert.equal(res.status, 400);
		const body = (await res.json()) as Record<string, unknown>;
		assert.match(String(body['error']), /unreachable/);
		assert.equal(countVersions('@acme/gone'), 0);
	});

	it('rejects a digest mismatch without writing', async () => {
		const bytes = indexBytes([descriptor(`sha256:${'aa'.repeat(32)}`, 'runtime', 'binary', 'libfoo.so')]);
		serveOrigin('oci.test', bytes);
		const res = await publish(
			'@acme/mismatch',
			'1.0.0',
			frame('@acme/mismatch', '1.0.0', REF, `sha256:${'bb'.repeat(32)}`)
		);
		assert.equal(res.status, 400);
		const body = (await res.json()) as Record<string, unknown>;
		assert.match(String(body['error']), /digest mismatch/);
		assert.equal(countVersions('@acme/mismatch'), 0);
	});

	it('rejects an oversize index without writing', async () => {
		const big = new Uint8Array(1024 * 1024 + 1);
		big.fill(120);
		serveOrigin('oci.test', big);
		const res = await publish(
			'@acme/big',
			'1.0.0',
			frame('@acme/big', '1.0.0', REF, `sha256:${'aa'.repeat(32)}`)
		);
		assert.equal(res.status, 400);
		const body = (await res.json()) as Record<string, unknown>;
		assert.match(String(body['error']), /1 MiB/);
		assert.equal(countVersions('@acme/big'), 0);
	});

	it('rejects a descriptor with a bad annotation', async () => {
		const bytes = indexBytes([descriptor(`sha256:${'aa'.repeat(32)}`, 'bindgen', 'binary', 'libfoo.so')]);
		serveOrigin('oci.test', bytes);
		const asserted = indexDigest(bytes);
		const res = await publish('@acme/badann', '1.0.0', frame('@acme/badann', '1.0.0', REF, asserted));
		assert.equal(res.status, 400);
		const body = (await res.json()) as Record<string, unknown>;
		assert.match(String(body['error']), /run\.rasmalai\.role/);
		assert.equal(countVersions('@acme/badann'), 0);
	});
});

describe('artifact pin storage and serving', () => {
	it('pins the digest, stores metadata, and echoes the pin', async () => {
		const bytes = indexBytes([
			descriptor(`sha256:${'aa'.repeat(32)}`, 'runtime', 'binary', 'libfoo.so', {
				os: 'linux',
				architecture: 'amd64'
			}),
			descriptor(`sha256:${'bb'.repeat(32)}`, 'wasm', 'binary', 'mod.wasm')
		]);
		const asserted = serveOrigin('oci.test', bytes);
		assert.ok(isArtifactDigest(asserted));
		const res = await publish('@acme/pinned', '1.0.0', frame('@acme/pinned', '1.0.0', REF, asserted));
		assert.equal(res.status, 201);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['artifactDigest'], asserted);
		const found = await getVersionIndexRow(fake.env, '@acme/pinned', '1.0.0');
		assert.ok(found);
		assert.equal(found.row.artifactDigest, asserted);
		const meta = JSON.parse(found.row.artifactMeta) as {
			ref: string;
			digest: string;
			entries: Array<{ digest: string; role: string; title: string }>;
		};
		assert.equal(meta.ref, REF);
		assert.equal(meta.digest, asserted);
		assert.deepEqual(
			meta.entries.map((e) => [e.role, e.title]),
			[
				['runtime', 'libfoo.so'],
				['wasm', 'mod.wasm']
			]
		);
		const metaRes = await readGET({
			params: { path: '@acme/pinned@1.0.0' },
			request: new Request('https://registry.test/api/v0/packages/@acme/pinned@1.0.0'),
			url: new URL('https://registry.test/api/v0/packages/@acme/pinned@1.0.0'),
			setHeaders: () => undefined,
			platform: { env: fake.env }
		} as never);
		assert.equal(metaRes.status, 200);
		const metaBody = (await metaRes.json()) as Record<string, unknown>;
		assert.equal(metaBody['artifactDigest'], asserted);
		const resolveRes = await resolvePOST({
			request: new Request('https://registry.test/api/v0/resolve', {
				method: 'POST',
				headers: { 'content-type': 'application/json' },
				body: JSON.stringify({ requirements: { '@acme/pinned': '*' } })
			}),
			url: new URL('https://registry.test/api/v0/resolve'),
			platform: { env: fake.env }
		} as never);
		assert.equal(resolveRes.status, 200);
		const resolved = (await resolveRes.json()) as {
			levels: Array<Array<Record<string, unknown>>>;
		};
		assert.equal(resolved.levels[0][0]['artifactDigest'], asserted);
	});

	it('publishes without artifacts and echoes null', async () => {
		const res = await publish('@acme/plain', '1.0.0', frame('@acme/plain', '1.0.0', null, null));
		assert.equal(res.status, 201);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['artifactDigest'], null);
		const found = await getVersionIndexRow(fake.env, '@acme/plain', '1.0.0');
		assert.ok(found);
		assert.equal(found.row.artifactDigest, '');
	});
});
