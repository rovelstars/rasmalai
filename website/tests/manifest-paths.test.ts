import { describe, it, before, after, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import {
	checkManifestEntries,
	checkManifestEntryName,
	chunkTarball,
	rebuildTarball,
	sha256Hex
} from '../src/lib/server/chunks.js';
import { POST as publishRoute } from '../src/routes/api/packages/+server.ts';
import { GET as packageRoute } from '../src/routes/api/packages/[...path]/+server.ts';
import {
	packageId,
	parsePackageName,
	publishPackage,
	putChunk,
	resolveGraph,
	storeTarManifest,
	RESOLVE_MAX_INLINE_BYTES,
	type PublishPayload
} from '../src/lib/server/db.js';

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

function tarEntry(name: string, data: Uint8Array): Uint8Array {
	const header = new Uint8Array(512);
	header.set(new TextEncoder().encode(name.slice(0, 100)));
	const oct = (off: number, len: number, value: number) => {
		const digits = value.toString(8);
		const pad = len - 1 - digits.length;
		for (let i = 0; i < pad; i++) header[off + i] = 48;
		for (let i = 0; i < digits.length; i++) header[off + pad + i] = digits.charCodeAt(i);
		header[off + len - 1] = 0;
	};
	oct(100, 8, 0o644);
	oct(108, 8, 0);
	oct(116, 8, 0);
	oct(124, 12, data.length);
	oct(136, 12, 0);
	for (let i = 148; i < 156; i++) header[i] = 32;
	header[156] = 48;
	header.set(new TextEncoder().encode('ustar\0'), 257);
	header.set(new TextEncoder().encode('00'), 263);
	header.set(new TextEncoder().encode('rnx'), 265);
	header.set(new TextEncoder().encode('rnx'), 297);
	let sum = 0;
	for (let i = 0; i < 512; i++) sum += header[i];
	const sumText = sum.toString(8).padStart(6, '0');
	for (let i = 0; i < 6; i++) header[148 + i] = sumText.charCodeAt(i);
	header[154] = 0;
	header[155] = 32;
	const blocks = Math.ceil(data.length / 512);
	const out = new Uint8Array(512 + blocks * 512);
	out.set(header);
	out.set(data, 512);
	return out;
}

const concat = (xs: Uint8Array[]): Uint8Array => {
	const out = new Uint8Array(xs.reduce((n, x) => n + x.length, 0));
	let off = 0;
	for (const x of xs) {
		out.set(x, off);
		off += x.length;
	}
	return out;
};

const UNSAFE_NAMES = [
	'../../etc/passwd',
	'..\\..\\evil',
	'C:\\evil',
	'/abs/path',
	'a/../../b'
];

describe('accepted manifest entry names', () => {
	it('accepts the ordinary names a tarball carries', () => {
		for (const name of ['a.txt', 'src/main.rnx', 'src/', 'a/b/c.txt', 'Project.config']) {
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

	it('keeps the empty name as the opaque tarball sentinel only', () => {
		assert.equal(
			checkManifestEntries([{ name: '', size: 10, dir: false, chunks: [] }]),
			null
		);
		const mixed = checkManifestEntries([
			{ name: '', size: 10, dir: false, chunks: [] },
			{ name: 'a.txt', size: 1, dir: false, chunks: [] }
		]);
		assert.ok(mixed);
		assert.equal(mixed.name, '');
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

async function publishTarball(
	name: string,
	version: string,
	tar: Uint8Array
): Promise<Response> {
	const request = new Request('https://registry.test/api/packages', {
		method: 'POST',
		headers: {
			authorization: 'Bearer preview-token',
			'content-type': 'application/octet-stream',
			'x-rnx-package-name': name,
			'x-rnx-package-version': version,
			'x-rnx-meta': JSON.stringify({ manifest: { deps: {} } })
		},
		body: tar as BodyInit
	});
	return publishRoute({
		request,
		url: new URL('https://registry.test/api/packages'),
		platform: { env: fake.env }
	} as never);
}

function get(path: string): Promise<Response> {
	return packageRoute({
		params: { path },
		url: new URL(`https://registry.test/api/packages/${path}`),
		platform: { env: fake.env },
		setHeaders: () => {}
	} as never);
}

describe('publish refuses an unsafe entry name', () => {
	it('rejects every escaping name and writes nothing', async () => {
		for (const unsafe of UNSAFE_NAMES) {
			const tar = concat([
				tarEntry('Project.config', new TextEncoder().encode('project: x\n')),
				tarEntry(unsafe, new TextEncoder().encode('pwned\n')),
				new Uint8Array(1024)
			]);
			const res = await publishTarball('@acme/unsafe', '1.0.0', tar);
			assert.equal(res.status, 400, unsafe);
			const body = (await res.json()) as Record<string, unknown>;
			assert.match(String(body['error']), /unsafe tar entry name/);
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
		const tar = concat([
			tarEntry('Project.config', new TextEncoder().encode('project: x\n')),
			tarEntry('src/main.rnx', new TextEncoder().encode('export fn main(): Int { return 1; }\n')),
			new Uint8Array(1024)
		]);
		const res = await publishTarball('@acme/safe', '1.0.0', tar);
		assert.equal(res.status, 201);
		const chunks = await get('@acme/safe@1.0.0/chunks');
		assert.equal(chunks.status, 200);
		const body = (await chunks.json()) as { entries: Array<{ name: string }> };
		assert.deepEqual(
			body.entries.map((e) => e.name),
			['Project.config', 'src/main.rnx']
		);
	});
});

describe('the chunk manifest route', () => {
	function payload(name: string, version: string): PublishPayload {
		return {
			name,
			version,
			description: 'fixture',
			author: 'test',
			license: 'MIT',
			tags: [],
			readme: '',
			docJson: '{"modules":[]}',
			checksum: `c-${name}-${version}`,
			manifestJson: '{}',
			tarballSha256: `sha-${name}-${version}`
		};
	}

	it('names a version with no file index instead of serving an empty one', async () => {
		await publishPackage(fake.env, payload('@acme/nofile', '1.0.0'));
		const res = await get('@acme/nofile@1.0.0/chunks');
		assert.equal(res.status, 404);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['code'], 'no-file-index');
		assert.match(String(body['message']), /ships no file index/);
	});

	it('varies on the encoding it serves', async () => {
		await publishPackage(fake.env, payload('@acme/vary', '1.0.0'));
		const parsed = parsePackageName('@acme/vary')!;
		await storeTarManifest(
			fake.env,
			`ver_${packageId(parsed.scope, parsed.name)}_1.0.0`,
			[{ name: 'a.txt', size: 1, dir: false, chunks: [] }]
		);
		const res = await get('@acme/vary@1.0.0/chunks');
		assert.equal(res.status, 200);
		assert.equal(res.headers.get('vary'), 'accept-encoding');
	});

	it('serves a chunk whose digest is written in upper case', async () => {
		await publishPackage(fake.env, payload('@acme/upper', '1.0.0'));
		const bytes = new TextEncoder().encode('chunk body');
		const hash = await sha256Hex(bytes);
		await putChunk(fake.env, hash, bytes.length, bytes);
		const res = await get(`@acme/upper@1.0.0/chunk/${hash.toUpperCase()}`);
		assert.equal(res.status, 200);
		assert.deepEqual(new Uint8Array(await res.arrayBuffer()), bytes);
		// The lower case spelling answers the same way.
		const lower = await get(`@acme/upper@1.0.0/chunk/${hash}`);
		assert.equal(lower.status, 200);
	});

	it('still refuses a digest that is not a digest', async () => {
		await publishPackage(fake.env, payload('@acme/badhash', '1.0.0'));
		const res = await get('@acme/badhash@1.0.0/chunk/not-a-hash');
		assert.equal(res.status, 404);
	});
});

describe('the inline budget', () => {
	const chunk = 'c'.repeat(64);

	async function publishClosure(full: string, entries: string): Promise<void> {
		await publishPackage(fake.env, {
			name: full,
			version: '1.0.0',
			description: 'fixture',
			author: 'test',
			license: 'MIT',
			tags: [],
			readme: '',
			docJson: '{"modules":[]}',
			checksum: 'c',
			manifestJson: '{}',
			tarballSha256: 'sha'
		});
		const parsed = parsePackageName(full)!;
		await storeTarManifest(
			fake.env,
			`ver_${packageId(parsed.scope, parsed.name)}_1.0.0`,
			JSON.parse(entries)
		);
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

describe('the chunk store still round-trips', () => {
	it('keeps the digest of a tarball with an ordinary name', async () => {
		const body = new TextEncoder().encode('hello world');
		const tar = concat([tarEntry('a.txt', body), new Uint8Array(1024)]);
		const { units, entries } = await chunkTarball(tar);
		const byHash = new Map(units.map((u) => [u.hash, u.bytes] as [string, Uint8Array]));
		assert.deepEqual(rebuildTarball(entries, byHash), tar);
	});
});
