import { describe, it, before, after, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import {
	checkManifestEntries,
	checkManifestEntryName,
	chunkCanonicalFiles,
	sha256Hex
} from '../src/lib/server/chunks.js';
import { POST as publishRoute } from '../src/routes/api/v0/packages/+server.ts';
import { GET as packageRoute } from '../src/routes/api/v0/packages/[...path]/+server.ts';
import { resolveGraph, RESOLVE_MAX_INLINE_BYTES } from '../src/lib/server/db.js';
import { seedPackage } from './publish-helpers.js';

// The manifest entry names are the paths a client writes to disk. The
// server refuses an unsafe name at publish, and the client refuses the same
// names at install; both sides apply the rule stated in
// `src/lib/server/chunks.ts`.

let fake: FakeLibsql;

before(async () => {
	fake = await startFakeLibsql();
});

after(async () => {
	await fake.close();
});

beforeEach(() => {
	fake.reset();
});

const UNSAFE_NAMES = [
	'../../etc/passwd',
	'..\\..\\evil',
	'C:\\evil',
	'/abs/path',
	'a/../../b'
];

function config(name: string, version: string, main = 'src/main.rnx'): string {
	return `export default {\n    project: {\n        name: "${name}",\n        version: "${version}"\n    },\n    entries: { main: "${main}" }\n}\n`;
}

describe('accepted manifest entry names', () => {
	it('accepts the ordinary names a publish carries', () => {
		for (const name of ['a.txt', 'src/main.rnx', 'a/b/c.txt', 'Project.config']) {
			assert.equal(checkManifestEntryName(name), null, name);
		}
	});

	it('refuses every name that escapes the destination directory', () => {
		for (const name of UNSAFE_NAMES) {
			assert.notEqual(checkManifestEntryName(name), null, name);
		}
	});

	it('refuses the shapes that only break on one platform', () => {
		const refused = [
			'',
			'/',
			'.',
			'..',
			'a/./b',
			'a//b',
			'C:evil',
			'c:/evil',
			'a\\b',
			'a\u0000b',
			'a\u0007b',
			'a\u007fb',
			'a\u009fb',
			'x'.repeat(256)
		];
		for (const name of refused) {
			assert.notEqual(checkManifestEntryName(name), null, JSON.stringify(name));
		}
	});

	it('refuses the empty name: there is no opaque-blob sentinel', () => {
		const alone = checkManifestEntries([{ name: '', size: 10, dir: false, chunks: [] }]);
		assert.ok(alone);
		assert.equal(alone.name, '');
	});

	it('refuses a manifest that carries an unsafe name', () => {
		const bad = checkManifestEntries([
			{ name: 'src/main.rnx', size: 1, dir: false, chunks: [] },
			{ name: '../../etc/passwd', size: 1, dir: false, chunks: [] }
		]);
		assert.ok(bad);
		assert.equal(bad.name, '../../etc/passwd');
		assert.match(bad.reason, /\.\./);
	});
});

async function publishFrame(name: string, version: string, files: Array<{ path: string; content: string }>): Promise<Response> {
	const request = new Request('https://registry.test/api/v0/packages', {
		method: 'POST',
		headers: {
			authorization: 'Bearer preview-token',
			'content-type': 'application/json',
			'x-rnx-protocol': '0'
		},
		body: JSON.stringify({
			name,
			version,
			files: [
				{ path: 'Project.config', content: config(name, version) },
				{ path: 'LICENSE', content: 'MIT License\n' },
				...files
			],
			description: 'fixture',
			readme: `# ${name}\n`,
			license: 'MIT'
		})
	});
	return publishRoute({
		request,
		url: new URL('https://registry.test/api/v0/packages'),
		platform: { env: fake.env }
	} as never);
}

function get(path: string): Promise<Response> {
	return packageRoute({
		params: { path },
		url: new URL(`https://registry.test/api/v0/packages/${path}`),
		platform: { env: fake.env },
		setHeaders: () => {}
	} as never);
}

describe('publish refuses an unsafe entry name', () => {
	it('rejects every escaping name and writes nothing', async () => {
		for (const unsafe of UNSAFE_NAMES) {
			const res = await publishFrame('@acme/unsafe', '1.0.0', [{ path: unsafe, content: 'pwned\n' }]);
			assert.equal(res.status, 400, unsafe);
			const body = (await res.json()) as Record<string, unknown>;
			assert.match(String(body['error']), /unsafe entry name/);
			assert.ok(String(body['error']).includes(unsafe), String(body['error']));
			// Nothing was written: the name is refused before the version is
			// stored.
			const rows = fake.db
				.prepare('SELECT count(*) AS c FROM package_versions')
				.all() as Array<{ c: number }>;
			assert.equal(Number(rows[0].c), 0, unsafe);
		}
	});

	it('accepts an ordinary name and serves the manifest it stored', async () => {
		const res = await publishFrame('@acme/safe', '1.0.0', [
			{ path: 'src/main.rnx', content: 'export fn main(): Int { return 1; }\n' }
		]);
		assert.equal(res.status, 201);
		const chunks = await get('@acme/safe@1.0.0/chunks');
		assert.equal(chunks.status, 200);
		const body = (await chunks.json()) as { entries: Array<{ name: string }> };
		assert.deepEqual(
			body.entries.map((e) => e.name),
			['LICENSE', 'Project.config', 'src/main.rnx']
		);
	});
});

describe('the chunk manifest route', () => {
	it('names a version with no file index instead of serving an empty one', async () => {
		await seedPackage(fake.env, '@acme/nofile', '1.0.0');
		const res = await get('@acme/nofile@1.0.0/chunks');
		assert.equal(res.status, 404);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['code'], 'no-file-index');
		assert.match(String(body['message']), /ships no file index/);
	});

	it('varies on the encoding it serves', async () => {
		await seedPackage(fake.env, '@acme/vary', '1.0.0', {
			entries: [{ name: 'a.txt', size: 1, dir: false, chunks: [] }]
		});
		const res = await get('@acme/vary@1.0.0/chunks');
		assert.equal(res.status, 200);
		assert.equal(res.headers.get('vary'), 'accept-encoding');
	});

	it('serves a chunk whose digest is written in upper case', async () => {
		const bytes = new TextEncoder().encode('chunk body');
		const hash = await sha256Hex(bytes);
		await seedPackage(fake.env, '@acme/upper', '1.0.0', {
			entries: [{ name: 'a.txt', size: bytes.length, dir: false, chunks: [hash] }],
			units: [{ hash, size: bytes.length, bytes }],
			manifestDigest: hash
		});
		const res = await get(`@acme/upper@1.0.0/chunk/${hash.toUpperCase()}`);
		assert.equal(res.status, 200);
		assert.deepEqual(new Uint8Array(await res.arrayBuffer()), bytes);
		// The lower case spelling answers the same way.
		const lower = await get(`@acme/upper@1.0.0/chunk/${hash}`);
		assert.equal(lower.status, 200);
	});

	it('still refuses a digest that is not a digest', async () => {
		await seedPackage(fake.env, '@acme/badhash', '1.0.0');
		const res = await get('@acme/badhash@1.0.0/chunk/not-a-hash');
		assert.equal(res.status, 404);
	});
});

describe('the inline budget', () => {
	const chunk = 'c'.repeat(64);

	async function publishClosure(full: string, entries: string): Promise<void> {
		await seedPackage(fake.env, full, '1.0.0', { entries: JSON.parse(entries) });
	}

	it('counts the response bytes, not the stored index', async () => {
		// A closure whose stored indexes fit the cap on their own, but whose
		// response (the indexes plus the node metadata and the per-node
		// wrapper) does not. The cap is about the response, so the answer is
		// a refusal.
		const count = 21;
		const packages = 100;
		const entries = JSON.stringify(
			Array.from({ length: count }, (_, i) => ({
				name: `src/f${i}.rnx`,
				size: 1,
				dir: false,
				chunks: [chunk]
			}))
		);
		const storedTotal = entries.length * packages;
		assert.ok(
			storedTotal < RESOLVE_MAX_INLINE_BYTES,
			`the stored indexes (${storedTotal}) must fit the cap for this to test the response size`
		);
		const reqs: Record<string, string> = {};
		for (let i = 0; i < packages; i++) {
			const full = `@acme/cap${i}`;
			reqs[full] = '*';
			await publishClosure(full, entries);
		}
		// The response without the entries, measured on the same closure.
		const plain = await resolveGraph(fake.env, reqs, []);
		const plainBytes = new TextEncoder().encode(
			JSON.stringify({ resolved: plain.resolved, base: '', levels: plain.levels })
		).length;
		assert.ok(
			plainBytes + storedTotal > RESOLVE_MAX_INLINE_BYTES,
			`the response (${plainBytes + storedTotal}) must be over the cap for this to test the response size`
		);
		const out = await resolveGraph(fake.env, reqs, [], {
			inlineChunks: true,
			advertiseChunks: true
		});
		assert.equal(out.chunksDeclined, 'size-cap-exceeded');
		assert.equal(out.chunks, 'links');
		for (const level of out.levels) for (const n of level) assert.equal(n.entries, undefined);
	});

	it('still inlines a closure whose response fits', async () => {
		const entries = JSON.stringify([
			{ name: 'Project.config', size: 10, dir: false, chunks: ['a'.repeat(64)] }
		]);
		await publishClosure('@acme/smallcap', entries);
		const out = await resolveGraph(fake.env, { '@acme/smallcap': '*' }, [], {
			inlineChunks: true,
			advertiseChunks: true
		});
		assert.equal(out.chunks, 'inline');
		assert.equal(out.chunksDeclined, null);
		assert.equal(out.levels[0][0].entries?.[0].name, 'Project.config');
	});
});

describe('canonical chunking still round-trips', () => {
	it('reassembles a large file from its content-defined parts', async () => {
		const big = new Uint8Array(200 * 1024);
		for (let off = 0; off < big.length; off += 65536) {
			crypto.getRandomValues(big.subarray(off, Math.min(off + 65536, big.length)));
		}
		const { units, entries } = await chunkCanonicalFiles([{ name: 'big.bin', bytes: big }]);
		assert.ok(units.length > 1);
		const byHash = new Map(units.map((u) => [u.hash, u.bytes] as [string, Uint8Array]));
		const parts: Uint8Array[] = [];
		for (const h of entries[0].chunks) parts.push(byHash.get(h)!);
		const total = parts.reduce((n, p) => n + p.length, 0);
		assert.equal(total, big.length);
		const out = new Uint8Array(total);
		let at = 0;
		for (const p of parts) {
			out.set(p, at);
			at += p.length;
		}
		assert.deepEqual(out, big);
	});
});
