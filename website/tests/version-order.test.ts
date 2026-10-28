import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { getPackage } from '../src/lib/server/db.js';
import { seedPackage } from './publish-helpers.js';
import { load as packageLoad } from '../src/routes/packages/[...slug]/+page.server.ts';

let fake: FakeLibsql;

before(async () => {
	fake = await startFakeLibsql();
});

after(async () => {
	await fake.close();
});

function pinCreatedAt(value: number) {
	fake.db.prepare('UPDATE package_versions SET created_at = ?').run(value);
}

function headers() {
	const seen: Record<string, string> = {};
	return {
		seen,
		setHeaders(h: Record<string, string>) {
			Object.assign(seen, h);
		}
	};
}

async function loadSlug(slug: string) {
	const h = headers();
	const data = (await packageLoad({
		params: { slug },
		platform: { env: fake.env },
		url: new URL(`https://registry.test/packages/${slug}`),
		setHeaders: h.setHeaders
	} as never)) as { active: { version: string } };
	return { data, cache: h.seen['Cache-Control'] };
}

describe('package version total order', () => {
	it('breaks same-second ties by semver regardless of insert order', async () => {
		fake.reset();
		await seedPackage(fake.env, '@acme/chrono', '1.0.0');
		await seedPackage(fake.env, '@acme/chrono', '1.1.0');
		await seedPackage(fake.env, '@acme/orhc', '1.1.0');
		await seedPackage(fake.env, '@acme/orhc', '1.0.0');
		pinCreatedAt(1700000000);
		const first = await getPackage(fake.env, '@acme/chrono');
		const second = await getPackage(fake.env, '@acme/orhc');
		assert.ok(first && second);
		assert.deepEqual(
			first.versions.map((v) => v.version),
			['1.1.0', '1.0.0']
		);
		assert.deepEqual(
			second.versions.map((v) => v.version),
			first.versions.map((v) => v.version)
		);
	});

	it('keeps floating short and pinned day windows on tied rows', async () => {
		for (const slug of ['@acme/chrono', '@acme/orhc']) {
			const floating = await loadSlug(slug);
			assert.equal(floating.data.active.version, '1.1.0');
			assert.equal(floating.cache, 'public, max-age=300, s-maxage=300');
			const pinned = await loadSlug(`${slug}@1.0.0`);
			assert.equal(pinned.data.active.version, '1.0.0');
			assert.equal(pinned.cache, 'public, max-age=86400, s-maxage=86400');
		}
	});

	it('ranks stable above its prerelease on tied rows', async () => {
		fake.reset();
		await seedPackage(fake.env, '@acme/pre', '1.0.0-beta.1');
		await seedPackage(fake.env, '@acme/pre', '1.0.0');
		pinCreatedAt(1700000001);
		const pkg = await getPackage(fake.env, '@acme/pre');
		assert.ok(pkg);
		assert.deepEqual(
			pkg.versions.map((v) => v.version),
			['1.0.0', '1.0.0-beta.1']
		);
		const floating = await loadSlug('@acme/pre');
		assert.equal(floating.data.active.version, '1.0.0');
	});
});
