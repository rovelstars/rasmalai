import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { seedPackage } from './publish-helpers.js';

// The migration ladder runs once per database, before the first query. This
// file starts from a database that predates the mutation count column, so
// the ladder has something to do.

let fake: FakeLibsql;

before(async () => {
	fake = await startFakeLibsql();
	// An older database: the column is not there yet, the schema is not
	// stamped, and the global generation counter is still in the meta table.
	// The ladder runs on the first query and has to deal with all of it.
	fake.db.exec('ALTER TABLE packages DROP COLUMN mutation_count');
	fake.db.exec('DELETE FROM meta');
	fake.db.prepare("INSERT INTO meta (key, value) VALUES ('registry_generation', '7')").run();
});

after(async () => {
	await fake.close();
});

describe('schema migration', () => {
	it('adds the mutation count column to an older database', async () => {
		const columns = fake.db
			.prepare("SELECT name FROM pragma_table_info('packages')")
			.all() as Array<{ name: string }>;
		assert.equal(
			columns.some((c) => c.name === 'mutation_count'),
			false,
			'the fixture database starts without the column'
		);
		// The first publish runs the ladder.
		await seedPackage(fake.env, '@old/pkg', '1.0.0');
		const after = fake.db
			.prepare("SELECT name FROM pragma_table_info('packages')")
			.all() as Array<{ name: string }>;
		assert.ok(
			after.some((c) => c.name === 'mutation_count'),
			'the ladder has to add the column'
		);
		const row = fake.db
			.prepare('SELECT mutation_count AS c FROM packages WHERE id = ?')
			.get('pkg_@old/pkg') as { c: number } | undefined;
		assert.equal(Number(row?.c), 1, 'the first publish starts the counter at one');
	});

	it('drops the global generation counter row', async () => {
		await seedPackage(fake.env, '@old/pkg3', '1.0.0');
		const rows = fake.db
			.prepare("SELECT count(*) AS c FROM meta WHERE key = 'registry_generation'")
			.all() as Array<{ c: number }>;
		assert.equal(Number(rows[0].c), 0, 'the ladder drops the global counter row');
		const stamped = fake.db
			.prepare("SELECT value AS v FROM meta WHERE key = 'schema_version'")
			.get() as { v: string } | undefined;
		assert.equal(Number(stamped?.v), 4, 'the ladder stamps the schema version');
	});

	it('carries the canonical columns on the version row', async () => {
		await seedPackage(fake.env, '@old/pkg4', '2.0.0', {
			entries: [{ name: 'a.txt', size: 1, dir: false, chunks: [] }],
			manifestDigest: 'd'.repeat(64),
			verdictKey: 'v'.repeat(64)
		});
		const row = fake.db
			.prepare(
				`SELECT v.manifest_digest AS d, v.verdict_key AS v FROM package_versions v
				 JOIN packages p ON p.id = v.package_id
				 WHERE p.scope = 'old' AND p.name = 'pkg4' AND v.version = '2.0.0'`
			)
			.get() as { d: string; v: string } | undefined;
		assert.equal(row?.d, 'd'.repeat(64));
		assert.equal(row?.v, 'v'.repeat(64));
		const tables = fake.db
			.prepare("SELECT name AS n FROM sqlite_master WHERE type = 'table'")
			.all() as Array<{ n: string }>;
		const names = tables.map((t) => t.n);
		assert.ok(names.includes('canonical_facts'), 'the fact cache table exists');
		assert.ok(names.includes('package_edges'), 'the edge table exists');
		assert.ok(!names.includes('manifest_chunks'), 'the mapping table is gone');
	});
});
