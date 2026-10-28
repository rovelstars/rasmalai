import { describe, it, before, after, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { POST as publishPOST, GET as listGET } from '../src/routes/api/v0/packages/+server.ts';
import { POST as resolvePOST } from '../src/routes/api/v0/resolve/+server.ts';
import {
	resetQuotaState,
	getVersionIndexRow,
	getChunkBytes,
	liveChunkHashes,
	yankVersion
} from '../src/lib/server/db.js';
import {
	checkRateLimit,
	resetRateLimits,
	rateLimitSize,
	RESOLVE_RATE_LIMIT,
	LIST_RATE_LIMIT
} from '../src/lib/server/rateLimit.js';
import { seedPackage } from './publish-helpers.js';

let fake: FakeLibsql;

function config(name: string, version: string): string {
	return `export default {\n    project: {\n        name: "${name}",\n        version: "${version}"\n    },\n    entries: { main: "src/main.rnx" }\n}\n`;
}

function frameOf(name: string, version: string, requestId = ''): Record<string, unknown> {
	return {
		name,
		version,
		files: [
			{ path: 'Project.config', content: config(name, version) },
			{ path: 'LICENSE', content: 'MIT License\n' },
			{ path: 'src/main.rnx', content: 'export fn main(): Void {\n  let x = 1;\n}\n' }
		],
		description: 'fixture',
		readme: `# ${name}\n`,
		license: 'MIT',
		requestId
	};
}

function publish(name: string, version: string, requestId = ''): Promise<Response> {
	const request = new Request('https://registry.test/api/v0/packages', {
		method: 'POST',
		headers: {
			'X-RNX-Protocol': '0',
			authorization: 'Bearer preview-token-123',
			'Content-Type': 'application/json'
		},
		body: JSON.stringify(frameOf(name, version, requestId))
	});
	return publishPOST({
		request,
		url: new URL('https://registry.test/api/v0/packages'),
		platform: { env: fake.env }
	} as never);
}

function resolve(body: unknown, ip: string | null): Promise<Response> {
	const headers: Record<string, string> = { 'content-type': 'application/json' };
	if (ip) headers['cf-connecting-ip'] = ip;
	return resolvePOST({
		request: new Request('https://registry.test/api/v0/resolve', {
			method: 'POST',
			headers,
			body: JSON.stringify(body)
		}),
		url: new URL('https://registry.test/api/v0/resolve'),
		platform: { env: fake.env }
	} as never);
}

function list(ip: string | null): Promise<Response> {
	const headers: Record<string, string> = {};
	if (ip) headers['cf-connecting-ip'] = ip;
	return listGET({
		request: new Request('https://registry.test/api/v0/packages', { headers }),
		platform: { env: fake.env },
		setHeaders: () => undefined,
		url: new URL('https://registry.test/api/v0/packages')
	} as never);
}

function countRows(table: string): number {
	const row = fake.db.prepare(`SELECT count(*) AS n FROM ${table}`).get() as { n: number };
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
	resetRateLimits();
	fake.reset();
});

describe('rate limiter unit', () => {
	it('allows the limit then refuses with a retry hint', () => {
		for (let i = 0; i < 5; i++) {
			assert.deepEqual(checkRateLimit('k', 5, 60_000, 1000 + i), { ok: true });
		}
		const verdict = checkRateLimit('k', 5, 60_000, 1005);
		assert.equal(verdict.ok, false);
		if (!verdict.ok) assert.equal(verdict.retryAfterSec, 60);
	});

	it('opens a fresh window after the interval', () => {
		assert.deepEqual(checkRateLimit('w', 1, 1000, 0), { ok: true });
		assert.equal(checkRateLimit('w', 1, 1000, 500).ok, false);
		assert.deepEqual(checkRateLimit('w', 1, 1000, 1000), { ok: true });
	});

	it('bounds its memory under key floods', () => {
		for (let i = 0; i < 5000; i++) checkRateLimit(`flood-${i}`, 1, 60_000, 0);
		assert.ok(rateLimitSize() <= 4096, `tracked ${rateLimitSize()}`);
	});
});

describe('resolve rate limit', () => {
	it('lets a legit resolve through and still resolves after the limit lands', async () => {
		await seedPackage(fake.env, '@acme/rl', '1.0.0');
		const ip = '10.7.0.1';
		const first = await resolve({ requirements: { '@acme/rl': '*' } }, ip);
		assert.equal(first.status, 200);
		for (let i = 1; i < RESOLVE_RATE_LIMIT; i++) {
			const res = await resolve({ requirements: { '@acme/rl': '*' } }, ip);
			assert.notEqual(res.status, 429);
		}
		const tripsBefore = fake.counters.roundTrips;
		const limited = await resolve({ requirements: { '@acme/rl': '*' } }, ip);
		assert.equal(limited.status, 429);
		const body = (await limited.json()) as Record<string, unknown>;
		assert.equal(body['code'], 'rate-limited');
		assert.match(String(body['message']), /resolve rate limit exceeded \(60\/minute per client\)/);
		assert.ok(limited.headers.get('Retry-After'));
		assert.equal(fake.counters.roundTrips, tripsBefore);
	});

	it('does not bucket requests with no attributable address', async () => {
		for (let i = 0; i < RESOLVE_RATE_LIMIT + 5; i++) {
			const res = await resolve({ requirements: { '@acme/missing': '*' } }, null);
			assert.notEqual(res.status, 429);
		}
	});
});

describe('package list rate limit', () => {
	it('serves the catalog under the limit and names the limit past it', async () => {
		await seedPackage(fake.env, '@acme/rl-list', '1.0.0');
		const ip = '10.7.0.2';
		const first = await list(ip);
		assert.equal(first.status, 200);
		const payload = (await first.json()) as { packages: unknown[] };
		assert.ok(payload.packages.some((p) => (p as { name: string }).name === '@acme/rl-list'));
		for (let i = 1; i < LIST_RATE_LIMIT; i++) {
			const res = await list(ip);
			assert.notEqual(res.status, 429);
		}
		const limited = await list(ip);
		assert.equal(limited.status, 429);
		const body = (await limited.json()) as Record<string, unknown>;
		assert.equal(body['code'], 'rate-limited');
		assert.match(String(body['message']), /package list rate limit exceeded \(120\/minute per client\)/);
		assert.ok(limited.headers.get('Retry-After'));
	});
});

describe('publish retry storage', () => {
	it('a retry with the same requestId stores no duplicate bytes', async () => {
		const one = await publish('@acme/retry', '1.0.0', 'req-dedup-1');
		assert.equal(one.status, 201);
		const chunks = countRows('chunks');
		const versions = countRows('package_versions');
		const audits = countRows('audit_log');
		assert.ok(chunks > 0);
		const two = await publish('@acme/retry', '1.0.0', 'req-dedup-1');
		assert.equal(two.status, 200);
		const dup = (await two.json()) as Record<string, unknown>;
		assert.equal(dup['duplicate'], true);
		assert.equal(countRows('chunks'), chunks);
		assert.equal(countRows('package_versions'), versions);
		assert.equal(countRows('audit_log'), audits);
	});
});

describe('yank storage lifecycle', () => {
	it('a yank keeps the bytes served and the hashes live', async () => {
		const res = await publish('@acme/yankbytes', '1.0.0', 'req-yank-1');
		assert.equal(res.status, 201);
		const found = await getVersionIndexRow(fake.env, '@acme/yankbytes', '1.0.0');
		assert.ok(found);
		const hashes = (JSON.parse(found.row.tarManifestJson) as Array<{ chunks: string[] }>).flatMap(
			(e) => e.chunks
		);
		assert.ok(hashes.length > 0);
		const yanked = await yankVersion(fake.env, '@acme/yankbytes', '1.0.0', 'req-yank-2');
		assert.equal(yanked.ok, true);
		for (const hash of hashes) {
			assert.ok(await getChunkBytes(fake.env, hash));
		}
		const live = await liveChunkHashes(fake.env);
		for (const hash of hashes) assert.ok(live.has(hash));
	});
});
