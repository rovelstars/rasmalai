import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { packagePointerUrls, purgeUrls } from '../src/lib/server/db.js';

describe('purge', () => {
	it('lists versioned sub-resources', () => {
		const urls = packagePointerUrls('https://example.com', '@acme/a', '1.0.0');
		assert.ok(urls.includes('https://example.com/api/packages'));
		assert.ok(urls.includes('https://example.com/api/packages/@acme/a'));
		assert.ok(urls.includes('https://example.com/packages'));
		assert.ok(urls.includes('https://example.com/packages/@acme/a'));
		assert.ok(urls.includes('https://example.com/api/packages/@acme/a@1.0.0'));
		assert.ok(urls.includes('https://example.com/api/packages/@acme/a@1.0.0/api'));
		assert.ok(urls.includes('https://example.com/api/packages/@acme/a@1.0.0/manifest'));
		assert.ok(urls.includes('https://example.com/api/packages/@acme/a@1.0.0/chunks'));
		assert.ok(urls.includes('https://example.com/api/packages/@acme/a@1.0.0/guides'));
		assert.ok(urls.includes('https://example.com/packages/@acme/a@1.0.0'));
	});

	it('never throws and reports verification', async () => {
		const r = await purgeUrls({}, ['https://example.com/api/packages'], { fullName: '@acme/a' });
		assert.equal(typeof r.ok, 'boolean');
		assert.equal(typeof r.verified, 'boolean');
		assert.equal(r.verified, false);
		const empty = await purgeUrls({}, []);
		assert.equal(empty.ok, true);
	});
});
