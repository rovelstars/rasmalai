import { describe, it, before, after, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { POST } from '../src/routes/api/v0/packages/[...path]/+server.ts';
import { seedPackage } from './publish-helpers.js';

// The yank and the takedown are the two mutations that change what a
// reader sees. Both purge the edge, and both only report success when
// the purge is verified.

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

function seed(name: string, version: string) {
	return seedPackage(fake.env, name, version);
}

function call(
	path: string,
	env: Record<string, string | undefined>,
	token = 'preview-token'
): Promise<Response> {
	const request = new Request(`https://registry.test/api/v0/packages/${path}`, {
		method: 'POST',
		headers: { authorization: `Bearer ${token}` },
		body: '{}'
	});
	return POST({
		params: { path },
		request,
		url: new URL(`https://registry.test/api/v0/packages/${path}`),
		platform: { env }
	} as never);
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

function stubFetch(
	purge: () => Response,
	edge: (url: string) => Response
): void {
	globalThis.fetch = (async (input: unknown) => {
		const url = String(input);
		if (url.includes('/purge_cache')) return purge();
		return edge(url);
	}) as unknown as typeof fetch;
}

describe('yank route', () => {
	it('reports the yank when no purge is configured', async () => {
		await seed('@acme/yankplain', '1.0.0');
		const res = await call('@acme/yankplain@1.0.0/yank', fake.env);
		assert.equal(res.status, 200);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['status'], 'yanked');
		const rows = fake.db
			.prepare("SELECT count(*) AS c FROM audit_log WHERE action = 'purge-skipped'")
			.all() as Array<{ c: number }>;
		assert.equal(Number(rows[0].c), 1);
	});

	it('does not report the yank while the edge serves the old copy', async () => {
		await seed('@acme/yankstuck', '1.0.0');
		const env = { ...fake.env, CF_ZONE_ID: 'zone', CF_PURGE_TOKEN: 'token' };
		stubFetch(
			() => json({ success: true }),
			() => edgeResponse({ status: 'live' }, 'HIT')
		);
		const res = await call('@acme/yankstuck@1.0.0/yank', env);
		assert.equal(res.status, 503);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['code'], 'unavailable');
		const rows = fake.db
			.prepare("SELECT count(*) AS c FROM audit_log WHERE action = 'purge-failed'")
			.all() as Array<{ c: number }>;
		assert.equal(Number(rows[0].c), 1);
	});

	it('reports the yank once the edge answers with the new state', async () => {
		await seed('@acme/yankok', '1.0.0');
		const env = { ...fake.env, CF_ZONE_ID: 'zone', CF_PURGE_TOKEN: 'token' };
		stubFetch(
			() => json({ success: true }),
			() => edgeResponse({ status: 'yanked' }, 'MISS')
		);
		const res = await call('@acme/yankok@1.0.0/yank', env);
		assert.equal(res.status, 200);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['status'], 'yanked');
	});
});

describe('takedown route', () => {
	it('reports the takedown once the edge answers 410', async () => {
		await seed('@acme/takedownok', '1.0.0');
		const env = { ...fake.env, CF_ZONE_ID: 'zone', CF_PURGE_TOKEN: 'token', ADMIN_TOKEN: 'admin' };
		stubFetch(
			() => json({ success: true }),
			() => edgeResponse({ code: 'withdrawn' }, 'MISS', 410)
		);
		const res = await call('@acme/takedownok@1.0.0/takedown', env, 'admin');
		assert.equal(res.status, 200);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['status'], 'tombstoned');
	});

	it('does not report the takedown while the edge serves the old copy', async () => {
		await seed('@acme/takedownstuck', '1.0.0');
		const env = { ...fake.env, CF_ZONE_ID: 'zone', CF_PURGE_TOKEN: 'token', ADMIN_TOKEN: 'admin' };
		stubFetch(
			() => json({ success: true }),
			() => edgeResponse({ status: 'live' }, 'MISS')
		);
		const res = await call('@acme/takedownstuck@1.0.0/takedown', env, 'admin');
		assert.equal(res.status, 503);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['code'], 'unavailable');
	});
});
