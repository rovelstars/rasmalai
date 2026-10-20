import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { publishPackage, type PublishPayload } from '../src/lib/server/db.js';

// The migration ladder runs once per database, before the first query. This
// file starts from a database that predates the mutation count column, so
// the ladder has something to do.

let fake: FakeLibsql;

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
		await publishPackage(fake.env, payload('@old/pkg', '1.0.0'));
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
		await publishPackage(fake.env, payload('@old/pkg3', '1.0.0'));
		const rows = fake.db
			.prepare("SELECT count(*) AS c FROM meta WHERE key = 'registry_generation'")
			.all() as Array<{ c: number }>;
		assert.equal(Number(rows[0].c), 0, 'the ladder drops the global counter row');
		const stamped = fake.db
			.prepare("SELECT value AS v FROM meta WHERE key = 'schema_version'")
			.get() as { v: string } | undefined;
		assert.equal(Number(stamped?.v), 3, 'the ladder stamps the schema version');
	});
});
