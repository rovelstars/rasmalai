import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startEmptyFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { seedPackage, seedPayload } from './publish-helpers.js';
import {
	clearVersionIndex,
	getChunkBytes,
	getVersionIndexRow,
	resolveGraph
} from '../src/lib/server/db.js';

// The self-host proof: a deployment that starts with an empty database
// must come up through the migration ladder alone, then serve a full
// publish-to-fetch round trip. No seed dump, no dashboard step, no
// production database is involved anywhere in this file.

let fake: FakeLibsql;

before(async () => {
	fake = await startEmptyFakeLibsql();
	const tables = fake.db
		.prepare("SELECT count(*) AS c FROM sqlite_master WHERE type = 'table'")
		.get() as { c: number };
	assert.equal(Number(tables.c), 0, 'the fixture database starts with no tables at all');
});

after(async () => {
	await fake.close();
});

describe('schema from zero', () => {
	it('migrates an empty database to the current schema on first use', async () => {
		await seedPackage(fake.env, '@acme/zero', '1.0.0');
		const version = fake.db
			.prepare("SELECT value AS v FROM meta WHERE key = 'schema_version'")
			.get() as { v: string } | undefined;
		assert.equal(version?.v, '6');
		for (const table of [
			'packages',
			'package_versions',
			'tombstones',
			'audit_log',
			'advisories',
			'chunks',
			'canonical_facts',
			'package_edges',
			'package_deps',
			'download_daily',
			'users',
			'sessions',
			'version_capabilities',
			'licenses'
		]) {
			const row = fake.db
				.prepare("SELECT count(*) AS c FROM sqlite_master WHERE type = 'table' AND name = ?")
				.get(table) as { c: number };
			assert.equal(Number(row.c), 1, `the ladder creates ${table}`);
		}
		const cols = fake.db
			.prepare("SELECT name AS n FROM pragma_table_info('packages')")
			.all() as Array<{ n: string }>;
		assert.ok(cols.some((c) => c.n === 'mutation_count'), 'the ladder backfills the token column');
	});

	it('serves a publish to fetch round trip on the fresh stack', async () => {
		clearVersionIndex();
		const bytes = new TextEncoder().encode('export fn main(): Void {\n  return;\n}\n');
		const { createHash } = await import('node:crypto');
		const hash = createHash('sha256').update(bytes).digest('hex');
		await seedPackage(
			fake.env,
			seedPayload('@acme/zero', '1.0.1', {
				manifestDigest: hash,
				entries: [{ name: 'src/main.rnx', size: bytes.length, dir: false, chunks: [hash] }],
				units: [{ hash, size: bytes.length, bytes }]
			})
		);
		const found = await getVersionIndexRow(fake.env, '@acme/zero', '1.0.1');
		assert.ok(found);
		assert.equal(found.row.manifestDigest, hash);
		const fetched = await getChunkBytes(fake.env, hash);
		assert.ok(fetched);
		assert.deepEqual(Buffer.from(fetched), Buffer.from(bytes));
		clearVersionIndex();
		const answer = await resolveGraph(fake.env, { '@acme/zero': '*' }, []);
		assert.equal(answer.resolved['@acme/zero'], '1.0.1');
		assert.equal(answer.levels[0][0].integrity, hash);
	});
});
