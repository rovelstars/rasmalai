import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { seedPackage } from './publish-helpers.js';
import { load as docsLoad } from '../src/routes/docs/[scope]/[name]/[...page]/+page.server.ts';

const DOC_JSON = JSON.stringify({
	modules: [
		{
			name: 'widgets',
			docs: { description: 'Widgets for every dashboard.', tags: [] },
			functions: [{ name: 'make', docs: { description: 'Build one.', tags: [] } }],
			classes: [],
			enums: [],
			constants: []
		}
	]
});

describe('package doc versions', () => {
	let fake: FakeLibsql;

	before(async () => {
		fake = await startFakeLibsql();
		await seedPackage(fake.env, '@acme/widgets', '2.0.0', { docJson: DOC_JSON });
		await seedPackage(fake.env, '@acme/widgets', '2.1.0', { docJson: DOC_JSON });
	});

	after(async () => {
		await fake.close();
	});

	function call(version?: string) {
		const url = new URL('https://registry.test/docs/@acme/widgets/overview');
		if (version) url.searchParams.set('v', version);
		return docsLoad({
			params: { scope: '@acme', name: 'widgets', page: 'overview' },
			platform: { env: fake.env },
			url,
			fetch: globalThis.fetch,
			setHeaders: () => {}
		} as never);
	}

	it('exposes the version list and selects the latest by default', async () => {
		const data = (await call()) as Record<string, unknown>;
		assert.equal(data['version'], '2.1.0');
		assert.equal(data['latest'], '2.1.0');
		assert.deepEqual(data['versions'], ['2.1.0', '2.0.0']);
	});

	it('serves a requested version and falls back to latest', async () => {
		const older = (await call('2.0.0')) as Record<string, unknown>;
		assert.equal(older['version'], '2.0.0');
		const fallback = (await call('9.9.9')) as Record<string, unknown>;
		assert.equal(fallback['version'], '2.1.0');
	});
});
