import { describe, it, before, after, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { packagePointerUrls, purgeUrls } from '../src/lib/server/db.js';

let fake: FakeLibsql;
const realFetch = globalThis.fetch;

before(async () => {
	fake = await startFakeLibsql();
});

after(async () => {
	await fake.close();
});

beforeEach(() => {
	fake.reset();
});

afterEach(() => {
	globalThis.fetch = realFetch;
});

interface FetchCall {
	url: string;
	method: string;
}

// The purge is two calls: the Cloudflare purge API and the read that checks
// the edge. The stub answers both.
function stubFetch(
	purge: () => Response | Promise<Response>,
	edge: (url: string) => Response | Promise<Response>
): FetchCall[] {
	const calls: FetchCall[] = [];
	globalThis.fetch = (async (input: unknown, init?: { method?: string }) => {
		const url = String(input);
		calls.push({ url, method: init?.method ?? 'GET' });
		if (url.includes('/purge_cache')) return purge();
		return edge(url);
	}) as unknown as typeof fetch;
	return calls;
}

function json(body: unknown, status = 200): Response {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

function edgeResponse(body: unknown, cacheStatus: string, status = 200): Response {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json', 'cf-cache-status': cacheStatus }
	});
}

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

	it('records the skipped purge with the verification state', async () => {
		const env = { ...fake.env };
		const r = await purgeUrls(env, ['https://example.com/api/packages'], { fullName: '@acme/skipped' });
		assert.equal(r.ok, true);
		assert.equal(r.verified, false);
		const rows = fake.db
			.prepare("SELECT details_json AS d FROM audit_log WHERE action = 'purge-skipped'")
			.all() as Array<{ d: string }>;
		assert.ok(rows.length > 0);
		const details = JSON.parse(rows[rows.length - 1].d) as { verified?: boolean };
		assert.equal(details.verified, false);
	});

	it('verifies the purge against the edge when the credentials are set', async () => {
		const env = { ...fake.env, CF_ZONE_ID: 'zone', CF_PURGE_TOKEN: 'token' };
		const calls = stubFetch(
			() => json({ success: true }),
			() => edgeResponse({ status: 'yanked' }, 'MISS')
		);
		const r = await purgeUrls(
			env,
			['https://example.com/api/packages/@acme/a@1.0.0'],
			{ fullName: '@acme/a', verify: { url: 'https://example.com/api/packages/@acme/a@1.0.0', bodyStatus: 'yanked' } }
		);
		assert.equal(r.ok, true);
		assert.equal(r.verified, true);
		// One purge call and one edge read.
		assert.equal(calls.filter((c) => c.url.includes('/purge_cache')).length, 1);
		assert.equal(calls.filter((c) => !c.url.includes('/purge_cache')).length, 1);
	});

	it('fails the verification when the edge still serves the old copy', async () => {
		const env = { ...fake.env, CF_ZONE_ID: 'zone', CF_PURGE_TOKEN: 'token' };
		const calls = stubFetch(
			() => json({ success: true }),
			() => edgeResponse({ status: 'live' }, 'HIT')
		);
		const r = await purgeUrls(
			env,
			['https://example.com/api/packages/@acme/a@1.0.0'],
			{ fullName: '@acme/a', verify: { url: 'https://example.com/api/packages/@acme/a@1.0.0', bodyStatus: 'yanked' } }
		);
		assert.equal(r.ok, false);
		assert.equal(r.verified, false);
		// The purge is retried to verified: three attempts.
		assert.equal(calls.filter((c) => c.url.includes('/purge_cache')).length, 3);
		const rows = fake.db
			.prepare("SELECT details_json AS d FROM audit_log WHERE action = 'purge-failed'")
			.all() as Array<{ d: string }>;
		assert.equal(rows.length, 1);
		const details = JSON.parse(rows[0].d) as { verified?: boolean };
		assert.equal(details.verified, false);
	});

	it('does not verify a withdrawn version the edge still answers 200 for', async () => {
		const env = { ...fake.env, CF_ZONE_ID: 'zone', CF_PURGE_TOKEN: 'token' };
		stubFetch(
			() => json({ success: true }),
			() => edgeResponse({ status: 'live' }, 'MISS')
		);
		const r = await purgeUrls(
			env,
			['https://example.com/api/packages/@acme/a@1.0.0'],
			{ fullName: '@acme/a', verify: { url: 'https://example.com/api/packages/@acme/a@1.0.0', status: 410 } }
		);
		assert.equal(r.ok, false);
		assert.equal(r.verified, false);
	});

	it('verifies a withdrawn version the edge answers 410 for', async () => {
		const env = { ...fake.env, CF_ZONE_ID: 'zone', CF_PURGE_TOKEN: 'token' };
		stubFetch(
			() => json({ success: true }),
			() => edgeResponse({ code: 'withdrawn' }, 'MISS', 410)
		);
		const r = await purgeUrls(
			env,
			['https://example.com/api/packages/@acme/a@1.0.0'],
			{ fullName: '@acme/a', verify: { url: 'https://example.com/api/packages/@acme/a@1.0.0', status: 410 } }
		);
		assert.equal(r.ok, true);
		assert.equal(r.verified, true);
	});

	it('does not verify a purge the Cloudflare API refused', async () => {
		const env = { ...fake.env, CF_ZONE_ID: 'zone', CF_PURGE_TOKEN: 'token' };
		stubFetch(
			() => json({ success: false }, 403),
			() => edgeResponse({ status: 'live' }, 'MISS')
		);
		const r = await purgeUrls(
			env,
			['https://example.com/api/packages/@acme/a@1.0.0'],
			{ fullName: '@acme/a', verify: { url: 'https://example.com/api/packages/@acme/a@1.0.0' } }
		);
		assert.equal(r.ok, false);
		assert.equal(r.verified, false);
	});
});
