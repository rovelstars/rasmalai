import { describe, it, before, after, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { POST } from '../src/routes/api/resolve/+server.ts';
import { publishPackage, clearVersionIndex, type PublishPayload } from '../src/lib/server/db.js';

// The route is the wire contract of the registry: an old client must keep
// the body it always got, and a client that asks for something the server
// cannot give has to be told in the body instead of guessing.

let fake: FakeLibsql;

function payload(name: string, version: string, overrides: Partial<PublishPayload> = {}): PublishPayload {
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
		tarballSha256: `sha-${name}-${version}`,
		...overrides
	};
}

function call(body: Record<string, unknown>): Promise<Response> {
	const request = new Request('https://registry.test/api/resolve', {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify(body)
	});
	return POST({
		request,
		url: new URL('https://registry.test/api/resolve'),
		platform: { env: fake.env }
	} as never);
}

before(async () => {
	fake = await startFakeLibsql();
});

after(async () => {
	await fake.close();
});

beforeEach(() => {
	clearVersionIndex();
	fake.reset();
});

describe('resolve route body', () => {
	it('answers a client that never opts in with the historical body', async () => {
		await publishPackage(fake.env, payload('@acme/wire', '1.0.0'));
		const res = await call({ requirements: { '@acme/wire': '*' } });
		assert.equal(res.status, 200);
		const body = (await res.json()) as Record<string, unknown>;
		assert.deepEqual(Object.keys(body), ['resolved', 'base', 'levels']);
		assert.equal('chunks' in body, false);
		assert.equal('chunksDeclined' in body, false);
		const levels = body['levels'] as Array<Array<Record<string, unknown>>>;
		assert.deepEqual(Object.keys(levels[0][0]), [
			'full',
			'version',
			'path',
			'integrity',
			'engineRange',
			'deps',
			'yanked'
		]);
	});

	it('names a declined inline answer instead of changing the body silently', async () => {
		const entries = JSON.stringify(
			Array.from({ length: 4000 }, (_, i) => ({
				name: `src/f${i}.rnx`,
				size: 1,
				dir: false,
				chunks: ['c'.repeat(64)]
			}))
		);
		await publishPackage(fake.env, payload('@acme/declined', '1.0.0'));
		fake.db
			.prepare('UPDATE package_versions SET tar_manifest_json = ? WHERE package_id = ?')
			.run(entries, 'pkg_@acme/declined');
		clearVersionIndex();
		const res = await call({ requirements: { '@acme/declined': '*' }, chunks: true });
		assert.equal(res.status, 200);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['chunks'], 'links');
		assert.equal(body['chunksDeclined'], 'size-cap-exceeded');
		const levels = body['levels'] as Array<Array<Record<string, unknown>>>;
		assert.equal('entries' in levels[0][0], false);
	});

	it('reports an inline answer it can give', async () => {
		await publishPackage(fake.env, payload('@acme/inlined', '1.0.0'));
		clearVersionIndex();
		const res = await call({ requirements: { '@acme/inlined': '*' }, chunks: true });
		assert.equal(res.status, 200);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['chunks'], 'inline');
		assert.equal('chunksDeclined' in body, false);
	});
});

describe('resolve route errors', () => {
	it('answers a budget failure with a structured body', async () => {
		const deps: Record<string, string> = {};
		for (let i = 0; i < 600; i++) deps[`@acme/n${i}`] = '*';
		await publishPackage(fake.env, payload('@acme/roombudget', '1.0.0', {
			manifestJson: JSON.stringify({ deps })
		}));
		clearVersionIndex();
		const res = await call({ requirements: { '@acme/roombudget': '*' } });
		assert.equal(res.status, 422);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['code'], 'budget-exceeded');
		assert.match(String(body['message']), /budget exceeded/);
		assert.equal(typeof body['requestId'], 'string');
	});

	it('answers an unknown package with a structured body', async () => {
		const res = await call({ requirements: { '@acme/ghost': '*' } });
		assert.equal(res.status, 404);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['code'], 'not-found');
		assert.match(String(body['message']), /does not exist/);
		assert.equal(typeof body['requestId'], 'string');
	});

	it('answers a malformed request with a structured body', async () => {
		const res = await call({ requirements: {} });
		assert.equal(res.status, 400);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['code'], 'bad-request');
		assert.equal(typeof body['message'], 'string');
		assert.equal(typeof body['requestId'], 'string');
	});

	it('answers a race with a retryable status instead of a wrong answer', async () => {
		await publishPackage(fake.env, payload('@acme/raceroute', '1.0.0'));
		clearVersionIndex();
		await call({ requirements: { '@acme/raceroute': '*' } });
		// A tombstone that reached the version while the row still says live:
		// every repair picks the same withdrawn version again.
		fake.db
			.prepare('INSERT INTO tombstones (package_id, version, reason, created_at) VALUES (?, ?, ?, ?)')
			.run('pkg_@acme/raceroute', '1.0.0', 'policy', 0);
		const res = await call({ requirements: { '@acme/raceroute': '*' } });
		assert.equal(res.status, 503);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['code'], 'unavailable');
		assert.match(String(body['message']), /raced/);
		assert.equal(typeof body['requestId'], 'string');
	});
});
