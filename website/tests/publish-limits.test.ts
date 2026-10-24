import { describe, it, before, after, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { POST as publishPOST } from '../src/routes/api/v0/packages/+server.ts';
import { GET as readGET } from '../src/routes/api/v0/packages/[...path]/+server.ts';
import {
	resetQuotaState,
	getVersionRow,
	getVersionIndexRow
} from '../src/lib/server/db.js';
import { seedPackage } from './publish-helpers.js';
import { planCaps } from '../src/lib/server/canonical.js';
import {
	capabilityScanCount,
	resetCapabilityScanCount
} from '../src/lib/server/permissions.js';

// The publish path is the one that writes, so every limit on it is
// checked against the same thing: what the request left behind. A refused
// publish must leave the store exactly as it found it.

let fake: FakeLibsql;

function config(name: string, version: string, permissions: string[] = []): string {
	const perms = permissions.length > 0 ? `,\n    permissions: [${permissions.map((p) => `"${p}"`).join(', ')}]` : '';
	return `export default {\n    project: {\n        name: "${name}",\n        version: "${version}"\n    },\n    entries: { main: "src/main.rnx" }${perms}\n}\n`;
}

function frameWithConfig(
	name: string,
	version: string,
	configText: string,
	files: Array<{ path: string; content: string }>
): Record<string, unknown> {
	return {
		name,
		version,
		files: [
			{ path: 'Project.config', content: configText },
			{ path: 'LICENSE', content: 'MIT License\n' },
			...files
		],
		description: 'fixture',
		readme: `# ${name}\n`,
		license: 'MIT'
	};
}

function frameOf(
	name: string,
	version: string,
	files: Array<{ path: string; content: string }>
): Record<string, unknown> {
	return {
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
	};
}

function pureMain(): Array<{ path: string; content: string }> {
	return [{ path: 'src/main.rnx', content: 'export fn main(): Void {\n  let x = 1;\n}\n' }];
}

function publish(
	name: string,
	version: string,
	frame: Record<string, unknown> | null,
	env: Record<string, string | undefined> = fake.env,
	headers: Record<string, string> = {}
): Promise<Response> {
	const request = new Request('https://registry.test/api/v0/packages', {
		method: 'POST',
		headers: {
			'X-RNX-Protocol': '0',
			authorization: 'Bearer preview-token-123',
			'Content-Type': 'application/json',
			...headers
		},
		body: JSON.stringify(frame ?? {})
	});
	return publishPOST({
		request,
		url: new URL('https://registry.test/api/v0/packages'),
		platform: { env }
	} as never);
}

function countRows(table: string): number {
	const row = fake.db.prepare(`SELECT count(*) AS n FROM ${table}`).get() as { n: number };
	return Number(row.n);
}

function countVersions(full: string): number {
	const row = fake.db
		.prepare(
			`SELECT count(*) AS n FROM package_versions v JOIN packages p ON p.id = v.package_id
			 WHERE ('@' || p.scope || '/' || p.name) = ? OR (p.scope = '' AND p.name = ?)`
		)
		.get(full, full) as { n: number };
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
	it('refuses a body over the cap on the declared length, before reading it', async () => {
		const caps = planCaps(fake.env);
		const res = await publish('@acme/toolarge', '1.0.0', frameOf('@acme/toolarge', '1.0.0', pureMain()), fake.env, {
			'content-length': String(caps.sourceBytes + caps.manifestBytes + 2 * 1024 * 1024 + 1)
		});
		assert.equal(res.status, 413);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['success'], false);
		assert.equal(fake.counters.roundTrips, 0);
		assert.equal(countRows('package_versions'), 0);
	});

	it('refuses a protocol its server does not speak', async () => {
		const request = new Request('https://registry.test/api/v0/packages', {
			method: 'POST',
			headers: {
				'X-RNX-Protocol': '2',
				authorization: 'Bearer preview-token-123',
				'Content-Type': 'application/json'
			},
			body: '{}'
		});
		const res = await publishPOST({
			request,
			url: new URL('https://registry.test/api/v0/packages'),
			platform: { env: fake.env }
		} as never);
		assert.equal(res.status, 400);
		const body = (await res.json()) as Record<string, unknown>;
		assert.match(String(body['error']), /X-RNX-Protocol: 0/);
		assert.equal(countRows('package_versions'), 0);
	});
});

describe('publish chunk cap', () => {
	it('refuses a package over the new-chunk limit and writes nothing', async () => {
		const caps = planCaps(fake.env);
		const files = [{ path: 'src/main.rnx', content: 'export fn main(): Int { return 0; }\n' }];
		for (let i = 0; i < caps.newChunks + 8; i++) {
			files.push({ path: `src/f${i}.rnx`, content: `export fn f${i}(): Int { return ${i}; }\n` });
		}
		const res = await publish('@acme/toomanychunks', '1.0.0', frameOf('@acme/toomanychunks', '1.0.0', files));
		assert.equal(res.status, 413);
		const body = (await res.json()) as Record<string, unknown>;
		assert.match(String(body['error']), /new-chunk limit/);
		assert.equal(countRows('package_versions'), 0);
		assert.equal(countRows('audit_log'), 0);
		assert.equal(countRows('chunks'), 0);
	});
});

describe('publish quota order', () => {
	it('refuses a doomed publish before it analyses and before it writes', async () => {
		const res = await publish(
			'@acme/overquota',
			'1.0.0',
			frameOf('@acme/overquota', '1.0.0', [
				{ path: 'src/main.rnx', content: 'export fn main(): Void {\n  print("hi");\n}\n' }
			]),
			{ ...fake.env, QUOTA_R2_BYTES: '100' }
		);
		assert.equal(res.status, 507);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['success'], false);
		assert.equal(body['error'], 'storage quota exceeded');
		// The analysis never ran and nothing was written.
		assert.equal(capabilityScanCount(), 0);
		assert.equal(countRows('package_versions'), 0);
		assert.equal(countRows('audit_log'), 0);
		assert.equal(countRows('chunks'), 0);
	});

	it('analyses and writes a publish that fits the quota', async () => {
		const res = await publish(
			'@acme/underquota',
			'1.0.0',
			frameOf('@acme/underquota', '1.0.0', pureMain()),
			{ ...fake.env, QUOTA_R2_BYTES: String(10 * 1024 * 1024) }
		);
		assert.equal(res.status, 201);
		assert.equal(countRows('package_versions'), 1);
		assert.equal(publishAuditRows(), 1);
	});

	it('refuses undeclared use in unreachable files, scanning them first', async () => {
		const res = await publish(
			'@acme/unreached',
			'1.0.0',
			frameOf('@acme/unreached', '1.0.0', [
				{ path: 'src/main.rnx', content: 'export fn main(): Void {}\n' },
				{ path: 'src/dead.rnx', content: 'export fn dead(): Void {\n  print("hi");\n}\n' }
			]),
			{ ...fake.env, QUOTA_R2_BYTES: String(10 * 1024 * 1024) }
		);
		assert.equal(res.status, 400);
		const body = (await res.json()) as Record<string, unknown>;
		assert.match(String(body['error']), /undeclared capabilities/);
		assert.equal(capabilityScanCount(), 1);
		assert.equal(countVersions('@acme/unreached'), 0);
	});

	it('counts a stored version against the budget', async () => {
		const contentFor = (tag: string) => `export fn ${tag}(): Void {\n  let x = 1;\n}`;
		const enc = new TextEncoder();
		const first = frameOf('@acme/budget1', '1.0.0', [{ path: 'src/main.rnx', content: contentFor('aaa') }]);
		const second = frameOf('@acme/budget2', '1.0.0', [{ path: 'src/main.rnx', content: contentFor('bbb') }]);
		const stored = Number(
			(fake.db.prepare('SELECT sum(size_bytes) AS n FROM chunks').get() as { n: number }).n ?? 0
		);
		// The budget counts stored canonical bytes: the redaction grows
		// Project.config by a fixed delta, so the raw frame size plus that
		// headroom brackets the first publish while two never fit.
		const firstBytes =
			enc.encode(config('@acme/budget1', '1.0.0') + 'MIT License\n' + contentFor('aaa')).length;
		const budget = stored + firstBytes + 15;
		const env = { ...fake.env, QUOTA_R2_BYTES: String(budget) };
		const one = await publish('@acme/budget1', '1.0.0', first, env);
		assert.equal(one.status, 201);
		const two = await publish('@acme/budget2', '1.0.0', second, env);
		assert.equal(two.status, 507);
	});

	it('does not count a refused publish against the budget', async () => {
		const content = 'export fn main(): Void {\n  print("hi");\n}';
		const frame = frameOf('@acme/budget3', '1.0.0', [{ path: 'src/main.rnx', content }]);
		const stored = Number(
			(fake.db.prepare('SELECT sum(size_bytes) AS n FROM chunks').get() as { n: number }).n ?? 0
		);
		const env = { ...fake.env, QUOTA_R2_BYTES: String(stored + 4096) };
		// Undeclared capability use fails the publish, and the budget is
		// untouched by it: the version was never stored.
		const refused = await publish('@acme/budget3', '1.0.0', frame, env);
		assert.equal(refused.status, 400);
		const good = frameOf('@acme/budget4', '1.0.0', pureMain());
		const after = await publish('@acme/budget4', '1.0.0', good, env);
		assert.equal(after.status, 201);
	});
});

describe('publish chunk write', () => {
	it('writes the chunk rows of a publish in one batch, addressed by hash', async () => {
		const one = frameOf('@acme/onechunk', '1.0.0', [
			{ path: 'src/main.rnx', content: 'export fn a(): Int { return 1; }\n' }
		]);
		const files = [{ path: 'src/main.rnx', content: 'export fn main(): Int { return 0; }\n' }];
		for (let i = 0; i < 11; i++) {
			files.push({ path: `src/f${i}.rnx`, content: `export fn f${i}(): Int { return ${i}; }\n` });
		}
		const frame = frameOf('@acme/batched', '1.0.0', files);

		fake.reset();
		const oneRes = await publish('@acme/onechunk', '1.0.0', one);
		assert.equal(oneRes.status, 201);
		const oneTrips = fake.counters.roundTrips;

		fake.reset();
		const res = await publish('@acme/batched', '1.0.0', frame);
		assert.equal(res.status, 201);
		const manyTrips = fake.counters.roundTrips;
		// Twelve chunks cost no more round trips than one: the rows go in
		// one batch.
		assert.ok(manyTrips <= oneTrips + 1, `one chunk ${oneTrips} round trips, twelve ${manyTrips}`);

		const rows = fake.db
			.prepare('SELECT hash, size_bytes FROM chunks ORDER BY hash')
			.all() as Array<{ hash: string; size_bytes: number }>;
		assert.ok(rows.length >= 12);
		for (const row of rows) {
			assert.match(row.hash, /^[0-9a-f]{64}$/);
			assert.ok(row.size_bytes > 0);
		}
		// The version manifest references exactly the stored chunk bytes.
		const version = await getVersionIndexRow(fake.env, '@acme/batched', '1.0.0');
		assert.ok(version);
		const entries = JSON.parse(version.row.tarManifestJson) as Array<{ chunks: string[] }>;
		const referenced = new Set(entries.flatMap((e) => e.chunks));
		assert.equal(referenced.size, 14);
		for (const hash of referenced) {
			assert.ok(rows.some((r) => r.hash === hash), hash);
		}
	});
});

describe('identical tree skips analysis', () => {
	it('republishes a version-only write with zero PUTs and the carried verdict', async () => {
		const files = [{ path: 'src/main.rnx', content: 'export fn main(): Void {\n  let x = 1;\n}\n' }];
		const one = await publish('@acme/same', '1.0.0', frameOf('@acme/same', '1.0.0', files));
		assert.equal(one.status, 201);
		const first = (await one.json()) as Record<string, unknown>;
		const chunksBefore = countRows('chunks');
		const factsBefore = countRows('canonical_facts');
		assert.ok(chunksBefore > 0);
		const two = await publish('@acme/same', '1.0.1', frameOf('@acme/same', '1.0.1', files));
		assert.equal(two.status, 201);
		const second = (await two.json()) as Record<string, unknown>;
		assert.equal(second['manifestDigest'], first['manifestDigest']);
		assert.equal(countRows('chunks'), chunksBefore);
		assert.equal(countRows('canonical_facts'), factsBefore);
		const v1 = await getVersionIndexRow(fake.env, '@acme/same', '1.0.0');
		const v2 = await getVersionIndexRow(fake.env, '@acme/same', '1.0.1');
		assert.ok(v1 && v2);
		assert.equal(v2.row.verdictKey, v1.row.verdictKey);
	});

	it('a reindent produces no new stored objects', async () => {
		const tidy = 'export fn main(): Void {\n  let x = 1;\n}\n';
		const messy = 'export fn   main():Void{let x=1;} // trailing\n';
		const one = await publish('@acme/reindent', '1.0.0', frameOf('@acme/reindent', '1.0.0', [{ path: 'src/main.rnx', content: tidy }]));
		assert.equal(one.status, 201);
		const chunksBefore = countRows('chunks');
		const two = await publish('@acme/reindent', '1.0.1', frameOf('@acme/reindent', '1.0.1', [{ path: 'src/main.rnx', content: messy }]));
		assert.equal(two.status, 201);
		const first = (await one.json()) as Record<string, unknown>;
		const second = (await two.json()) as Record<string, unknown>;
		assert.equal(second['manifestDigest'], first['manifestDigest']);
		assert.equal(countRows('chunks'), chunksBefore);
	});
});

describe('cross-file capability analysis', () => {
	it('rejects a sink reachable only through another file', async () => {
		const main = 'import { helper } from "./util";\nfn Main(): Int { return helper(); }\n';
		const util = 'export fn helper(): Int { print(41); return 41; }\n';
		const frame = frameWithConfig(
			'@acme/xfile',
			'1.0.0',
			config('@acme/xfile', '1.0.0'),
			[
				{ path: 'src/main.rnx', content: main },
				{ path: 'src/util.rnx', content: util }
			]
		);
		const res = await publish('@acme/xfile', '1.0.0', frame);
		assert.equal(res.status, 400);
		const body = (await res.json()) as Record<string, unknown>;
		assert.match(String(body['error']), /undeclared capabilities/);
		assert.match(String(body['error']), /term/);
		assert.equal(countVersions('@acme/xfile'), 0);
	});

	it('accepts the same tree when the ceiling covers the cross-file use', async () => {
		const main = 'import { helper } from "./util";\nfn Main(): Int { return helper(); }\n';
		const util = 'export fn helper(): Int { print(41); return 41; }\n';
		const frame = frameWithConfig(
			'@acme/xfileok',
			'1.0.0',
			config('@acme/xfileok', '1.0.0', ['term:write']),
			[
				{ path: 'src/main.rnx', content: main },
				{ path: 'src/util.rnx', content: util }
			]
		);
		const res = await publish('@acme/xfileok', '1.0.0', frame);
		assert.equal(res.status, 201);
	});
	it('accepts a package with a docs dir and guide files', async () => {
		const cfg = 'export default {\n    project: {\n        name: "@acme/withdocs",\n        version: "1.0.0"\n    },\n    entries: { main: "src/main.rnx", docs: "docs" }\n}\n';
		const frame = frameWithConfig('@acme/withdocs', '1.0.0', cfg, [
			{ path: 'src/main.rnx', content: 'export fn main(): Void {}\n' },
			{ path: 'docs/guide.md', content: '# Guide\n' }
		]);
		const res = await publish('@acme/withdocs', '1.0.0', frame);
		assert.equal(res.status, 201);
	});
});

describe('read projections', () => {	it('serves the same bytes from the narrow projection as from the wide one', async () => {
		const res = await publish(
			'@acme/projection',
			'1.0.0',
			frameOf('@acme/projection', '1.0.0', [
				{ path: 'src/main.rnx', content: 'export fn main(): Void {}\n' },
				{ path: 'src/other.rnx', content: 'export fn other(): Int { return 1; }\n' }
			])
		);
		assert.equal(res.status, 201);

		const wide = await getVersionRow(fake.env, '@acme/projection', '1.0.0');
		const narrow = await getVersionIndexRow(fake.env, '@acme/projection', '1.0.0');
		assert.ok(wide && narrow);
		assert.equal(narrow.row.tarManifestJson, wide.row.tarManifestJson);
		assert.equal(narrow.row.manifestJson, wide.row.manifestJson);
		assert.equal(narrow.row.guidesJson, wide.row.guidesJson);
		assert.equal(narrow.row.checksum, wide.row.checksum);
		assert.equal(narrow.row.manifestDigest, wide.row.manifestDigest);
		assert.equal(narrow.row.engineRange, wide.row.engineRange);
		assert.equal(narrow.row.status, wide.row.status);
		assert.equal(narrow.row.createdAt, wide.row.createdAt);
		assert.equal('docJson' in narrow.row, false);
	});

	it('serves the file index without reading the doc payload', async () => {
		await seedPackage(fake.env, '@acme/docs', '1.0.0', {
			docJson: JSON.stringify({ modules: [], filler: 'z'.repeat(200 * 1024) })
		});
		const request = new Request('https://registry.test/api/v0/packages/@acme/docs@1.0.0/chunks');
		fake.reset();
		const res = await readGET({
			params: { path: '@acme/docs@1.0.0/chunks' },
			request,
			url: new URL('https://registry.test/api/v0/packages/@acme/docs@1.0.0/chunks'),
			setHeaders: () => undefined,
			platform: { env: fake.env }
		} as never);
		assert.equal(res.status, 404);
		// The doc payload was never read: the narrow projection leaves it
		// on the table.
		assert.ok(fake.counters.bytesRead < 200 * 1024, `bytes read ${fake.counters.bytesRead}`);
	});
});
