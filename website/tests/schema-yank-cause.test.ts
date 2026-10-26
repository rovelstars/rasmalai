import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { seedPackage } from './publish-helpers.js';

// Regression: production stamped schema 6 predates the yank_cause column,
// so the selects added with it 500 every version read. The ladder must add
// the column and restamp on first write.

let fake: FakeLibsql;

before(async () => {
	fake = await startFakeLibsql();
	fake.db.exec('ALTER TABLE package_versions DROP COLUMN yank_cause');
	fake.db.exec('DELETE FROM meta');
	fake.db.prepare("INSERT INTO meta (key, value) VALUES ('schema_version', '6')").run();
});

after(async () => {
	await fake.close();
});

describe('yank_cause migration from schema 6', () => {
	it('adds the column and restamps to 7 on first write', async () => {
		await seedPackage(fake.env, '@old/yankmig', '1.0.0');
		const cols = fake.db
			.prepare("SELECT name FROM pragma_table_info('package_versions')")
			.all() as Array<{ name: string }>;
		assert.ok(
			cols.some((c) => c.name === 'yank_cause'),
			'the ladder has to add the column'
		);
		const stamped = fake.db
			.prepare("SELECT value AS v FROM meta WHERE key = 'schema_version'")
			.get() as { v: string } | undefined;
		assert.equal(Number(stamped?.v), 7, 'the ladder stamps the new schema version');
	});

	it('serves yank_cause on the version read shape', async () => {
		const row = fake.db
			.prepare(
				`SELECT v.yank_cause AS y FROM package_versions v
				 JOIN packages p ON p.id = v.package_id
				 WHERE p.scope = 'old' AND p.name = 'yankmig' AND v.version = '1.0.0'`
			)
			.get() as { y: string } | undefined;
		assert.equal(row?.y, '', 'un-yanked versions read back empty cause');
	});
});
