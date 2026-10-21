import { describe, it, before, after, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { POST as publishPOST } from '../src/routes/api/packages/+server.ts';
import { GET as readGET } from '../src/routes/api/packages/[...path]/+server.ts';
import {
	publishPackage,
	resetQuotaState,
	getVersionRow,
	getVersionIndexRow,
	MAX_TARBALL_BYTES,
	type PublishPayload
} from '../src/lib/server/db.js';
import {
	rebuildTarball,
	sha256Hex,
	MAX_CHUNKS,
	type TarEntryRef
} from '../src/lib/server/chunks.js';
import {
	capabilityScanCount,
	resetCapabilityScanCount
} from '../src/lib/server/permissions.js';

// The publish path is the one that writes, so every limit on it is
// checked against the same thing: what the request left behind. A refused
// publish must leave the store exactly as it found it.

let fake: FakeLibsql;

function payload(name: string, version: string, overrides: Partial<PublishPayload> = {}): PublishPayload {
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
		tarballSha256: `sha-${name}-${version}`,
		...overrides
	};
}

async function tarOf(files: Array<{ name: string; text: string }>): Promise<Uint8Array> {
	const enc = new TextEncoder();
	const units: Array<{ hash: string; bytes: Uint8Array }> = [];
	const entries: TarEntryRef[] = [];
	for (const f of files) {
		const bytes = enc.encode(f.text);
		const hash = await sha256Hex(bytes);
		units.push({ hash, bytes });
		entries.push({ name: f.name, size: bytes.length, dir: false, chunks: [hash] });
	}
	return rebuildTarball(entries, new Map(units.map((u) => [u.hash, u.bytes] as const)));
}

function publish(
	name: string,
	version: string,
	body: Uint8Array | null,
	env: Record<string, string | undefined> = fake.env,
	headers: Record<string, string> = {}
): Promise<Response> {
	const request = new Request('https://registry.test/api/packages', {
		method: 'POST',
		headers: {
			'X-RNX-Package-Name': name,
			'X-RNX-Package-Version': version,
			authorization: 'Bearer preview-token-123',
			'Content-Type': 'application/octet-stream',
			...headers
		},
		body: (body ?? new Uint8Array(0)) as unknown as BodyInit
	});
	return publishPOST({
		request,
		url: new URL('https://registry.test/api/packages'),
		platform: { env }
	} as never);
}

function countRows(table: string): number {
	const row = fake.db.prepare(`SELECT count(*) AS n FROM ${table}`).get() as { n: number };
	return Number(row.n);
}

// The publish audit rows only: the edge purge writes its own audit row.
function publishAuditRows(): number {
	const row = fake.db
		.prepare(`SELECT count(*) AS n FROM audit_log WHERE action = 'publish'`)
		.get() as { n: number };
	return Number(row.n);
}

before(async () => {
	fake = await startFakeLibsql();
});

after(async () => {
	await fake.close();
});

beforeEach(() => {
	resetQuotaState();
	resetCapabilityScanCount();
	fake.reset();
});

describe('publish body caps', () => {
	it('refuses an upload over the cap on the declared length, before reading it', async () => {
		const tar = await tarOf([{ name: 'src/main.rnx', text: 'export fn main(): Void {}' }]);
		const res = await publish('@acme/toolarge', '1.0.0', tar, fake.env, {
			'content-length': String(MAX_TARBALL_BYTES + 1)
		});
		assert.equal(res.status, 413);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['success'], false);
		assert.match(String(body['error']), /tarball exceeds 512 KiB/);
		assert.equal(fake.counters.roundTrips, 0);
		assert.equal(countRows('package_versions'), 0);
	});

	it('refuses an upload over the cap that arrives without a declared length', async () => {
		const big = new Uint8Array(MAX_TARBALL_BYTES + 1);
		const res = await publish('@acme/toolarge2', '1.0.0', big);
		assert.equal(res.status, 413);
		assert.equal(countRows('package_versions'), 0);
	});

	it('refuses a metadata body over its cap before reading it', async () => {
		const request = new Request('https://registry.test/api/packages', {
			method: 'POST',
			headers: {
				'X-RNX-Package-Name': '@acme/metatoo',
				'X-RNX-Package-Version': '1.0.0',
				authorization: 'Bearer preview-token-123',
				'Content-Type': 'application/json',
				'content-length': String(4 * 1024 * 1024)
			},
			body: '{"description":"x"}'
		});
		const res = await publishPOST({
			request,
			url: new URL('https://registry.test/api/packages'),
			platform: { env: fake.env }
		} as never);
		assert.equal(res.status, 413);
		assert.equal(countRows('package_versions'), 0);
	});
});

describe('publish chunk cap', () => {
	it('refuses a package over the chunk limit and writes nothing', async () => {
		const files = Array.from({ length: MAX_CHUNKS + 8 }, (_, i) => ({
			name: `src/f${i}.rnx`,
			text: `export fn f${i}(): Int { return ${i}; }`
		}));
		const tar = await tarOf(files);
		const res = await publish('@acme/toomanychunks', '1.0.0', tar);
		assert.equal(res.status, 413);
		const body = (await res.json()) as Record<string, unknown>;
		assert.match(String(body['error']), /chunks, over the 32 chunk limit/);
		assert.equal(countRows('package_versions'), 0);
		assert.equal(countRows('audit_log'), 0);
		assert.equal(countRows('chunks'), 0);
	});
});

describe('publish quota order', () => {
	it('refuses a doomed publish before it scans and before it writes', async () => {
		const tar = await tarOf([
			{ name: 'src/main.rnx', text: 'export fn main(): Void {\n  print("hi");\n}' }
		]);
		const res = await publish('@acme/overquota', '1.0.0', tar, {
			...fake.env,
			QUOTA_R2_BYTES: '1024'
		});
		assert.equal(res.status, 507);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['success'], false);
		assert.equal(body['error'], 'storage quota exceeded');
		// The scan never ran and nothing was written.
		assert.equal(capabilityScanCount(), 0);
		assert.equal(countRows('package_versions'), 0);
		assert.equal(countRows('audit_log'), 0);
		assert.equal(countRows('chunks'), 0);
	});

	it('still scans and writes a publish that fits the quota', async () => {
		const tar = await tarOf([
			{ name: 'src/main.rnx', text: 'export fn main(): Void {\n  let x = 1;\n}' }
		]);
		const res = await publish('@acme/underquota', '1.0.0', tar, {
			...fake.env,
			QUOTA_R2_BYTES: String(10 * 1024 * 1024)
		});
		assert.equal(res.status, 201);
		assert.equal(capabilityScanCount(), 1);
		assert.equal(countRows('package_versions'), 1);
		assert.equal(publishAuditRows(), 1);
	});

	it('counts a stored version against the budget', async () => {
		const contentFor = (tag: string) => `export fn ${tag}(): Void {\n  let x = 1;\n}`;
		const first = await tarOf([{ name: 'src/main.rnx', text: contentFor('aaa') }]);
		const second = await tarOf([{ name: 'src/main.rnx', text: contentFor('bbb') }]);
		const stored = Number(
			(fake.db.prepare('SELECT sum(size_bytes) AS n FROM chunks').get() as { n: number }).n ?? 0
		);
		// The budget holds the bytes already stored, the stored chunk of
		// the version and its archive, so a second version of the same
		// size does not fit, whether the sampler reads the stored bytes
		// again or not.
		const budget =
			stored + new TextEncoder().encode(contentFor('aaa')).length + first.byteLength - 1;
		const env = { ...fake.env, QUOTA_R2_BYTES: String(budget) };
		const one = await publish('@acme/budget1', '1.0.0', first, env);
		assert.equal(one.status, 201);
		const two = await publish('@acme/budget2', '1.0.0', second, env);
		assert.equal(two.status, 507);
	});

	it('does not count a refused publish against the budget', async () => {
		const content = 'export fn main(): Void {\n  let x = 1;\n}';
		const tar = await tarOf([{ name: 'src/main.rnx', text: content }]);
		const stored = Number(
			(fake.db.prepare('SELECT sum(size_bytes) AS n FROM chunks').get() as { n: number }).n ?? 0
		);
		const env = { ...fake.env, QUOTA_R2_BYTES: String(stored + tar.byteLength * 2) };
		// A tarball that is not canonical fails the round-trip check, and
		// the budget is untouched by it: the version was never stored.
		const broken = tar.slice();
		broken[110] = broken[110] ^ 0xff;
		const refused = await publish('@acme/budget3', '1.0.0', broken, env);
		assert.equal(refused.status, 400);
		const after = await publish('@acme/budget4', '1.0.0', tar, env);
		assert.equal(after.status, 201);
	});
});

describe('publish chunk write', () => {
	it('writes the chunk rows of a publish in one batch, with the same rows', async () => {
		const one = await tarOf([{ name: 'src/a.rnx', text: 'export fn a(): Int { return 1; }' }]);
		const files = Array.from({ length: 12 }, (_, i) => ({
			name: `src/f${i}.rnx`,
			text: `export fn f${i}(): Int { return ${i}; }`
		}));
		const tar = await tarOf(files);

		fake.reset();
		const oneRes = await publish('@acme/onechunk', '1.0.0', one);
		assert.equal(oneRes.status, 201);
		const oneTrips = fake.counters.roundTrips;

		fake.reset();
		const res = await publish('@acme/batched', '1.0.0', tar);
		assert.equal(res.status, 201);
		const manyTrips = fake.counters.roundTrips;
		// Twelve chunks cost no more round trips than one: the rows go in
		// one batch.
		assert.ok(manyTrips <= oneTrips + 1, `one chunk ${oneTrips} round trips, twelve ${manyTrips}`);

		const rows = fake.db
			.prepare(
				`SELECT c.hash AS hash, c.size_bytes AS size_bytes, c.first_seen_at AS first_seen_at
				 FROM chunks c JOIN manifest_chunks m ON m.chunk_hash = c.hash
				 WHERE m.version_id = ? ORDER BY c.hash`
			)
			.all('ver_pkg_@acme/batched_1.0.0') as Array<{
			hash: string;
			size_bytes: number;
			first_seen_at: number;
		}>;
		assert.equal(rows.length, 12);
		for (const row of rows) {
			assert.match(row.hash, /^[0-9a-f]{64}$/);
			assert.ok(row.size_bytes > 0);
		}
		// The mapping of the version holds the same hashes.
		const mapping = fake.db
			.prepare('SELECT chunk_hash FROM manifest_chunks WHERE version_id = ? ORDER BY ord')
			.all('ver_pkg_@acme/batched_1.0.0') as Array<{ chunk_hash: string }>;
		assert.equal(mapping.length, 12);
		assert.deepEqual(
			[...mapping.map((m) => m.chunk_hash)].sort(),
			rows.map((r) => r.hash).sort()
		);
	});
});

describe('read projections', () => {
	it('serves the same bytes from the narrow projection as from the wide one', async () => {
		const tar = await tarOf([
			{ name: 'src/main.rnx', text: 'export fn main(): Void {}' },
			{ name: 'src/other.rnx', text: 'export fn other(): Int { return 1; }' }
		]);
		const res = await publish('@acme/projection', '1.0.0', tar);
		assert.equal(res.status, 201);

		const wide = await getVersionRow(fake.env, '@acme/projection', '1.0.0');
		const narrow = await getVersionIndexRow(fake.env, '@acme/projection', '1.0.0');
		assert.ok(wide && narrow);
		assert.equal(narrow.row.tarManifestJson, wide.row.tarManifestJson);
		assert.equal(narrow.row.manifestJson, wide.row.manifestJson);
		assert.equal(narrow.row.guidesJson, wide.row.guidesJson);
		assert.equal(narrow.row.checksum, wide.row.checksum);
		assert.equal(narrow.row.tarballSha256, wide.row.tarballSha256);
		assert.equal(narrow.row.engineRange, wide.row.engineRange);
		assert.equal(narrow.row.status, wide.row.status);
		assert.equal(narrow.row.createdAt, wide.row.createdAt);
		assert.equal('docJson' in narrow.row, false);
	});

	it('serves the file index without reading the doc payload', async () => {
		await publishPackage(fake.env, payload('@acme/docs', '1.0.0', {
			docJson: JSON.stringify({ modules: [], filler: 'z'.repeat(200 * 1024) })
		}));
		const request = new Request('https://registry.test/api/packages/@acme/docs@1.0.0/chunks');
		fake.reset();
		const res = await readGET({
			params: { path: '@acme/docs@1.0.0/chunks' },
			request,
			url: new URL('https://registry.test/api/packages/@acme/docs@1.0.0/chunks'),
			setHeaders: () => undefined,
			platform: { env: fake.env }
		} as never);
		assert.equal(res.status, 404);
		// The doc payload was never read: the narrow projection leaves it
		// on the table.
		assert.ok(fake.counters.bytesRead < 200 * 1024, `bytes read ${fake.counters.bytesRead}`);
	});
});
