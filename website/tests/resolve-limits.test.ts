import { describe, it, before, after, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { POST } from '../src/routes/api/v0/resolve/+server.ts';
import {
	clearVersionIndex,
	resolveCounters,
	resetResolveCounters,
	resolveGraph,
	RESOLVE_MAX_SUBREQUESTS,
	type HaveEntry
} from '../src/lib/server/db.js';
import { seedPackage, seedPayload, type CanonicalPublishPayload } from './publish-helpers.js';

// The resolve endpoint is anonymous. Every limit here is a guard against
// a request that costs the server more than it answers, so the tests hold
// two things true at once: the refusal is a named status, and the refused
// request spends nothing.

let fake: FakeLibsql;

function payload(name: string, version: string, overrides: Partial<CanonicalPublishPayload> = {}): CanonicalPublishPayload {
	return seedPayload(name, version, overrides);
}

function call(body: unknown, headers: Record<string, string> = {}): Promise<Response> {
	const request = new Request('https://registry.test/api/v0/resolve', {
		method: 'POST',
		headers: { 'content-type': 'application/json', ...headers },
		body: typeof body === 'string' ? body : JSON.stringify(body)
	});
	return POST({
		request,
		url: new URL('https://registry.test/api/v0/resolve'),
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
	resetResolveCounters();
	fake.reset();
});

describe('resolve request caps', () => {
	it('refuses a body over the cap before it is parsed', async () => {
		const big = JSON.stringify({ requirements: { '@acme/x': '*' }, pad: 'y'.repeat(70 * 1024) });
		const res = await call(big);
		assert.equal(res.status, 413);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['code'], 'payload-too-large');
		assert.match(String(body['message']), /body over 65536 bytes/);
		// The body was refused before a single store round trip.
		assert.equal(fake.counters.roundTrips, 0);
	});

	it('refuses a body over the cap on the declared length alone', async () => {
		const res = await call(JSON.stringify({ requirements: { '@acme/x': '*' } }), {
			'content-length': String(1024 * 1024)
		});
		assert.equal(res.status, 413);
		assert.equal(fake.counters.roundTrips, 0);
	});

	it('refuses a requirements map over the cap', async () => {
		const requirements: Record<string, string> = {};
		for (let i = 0; i < 513; i++) requirements[`@acme/p${i}`] = '*';
		const res = await call({ requirements });
		assert.equal(res.status, 413);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['code'], 'payload-too-large');
		assert.match(String(body['message']), /requirements map over 512 entries/);
		assert.equal(fake.counters.roundTrips, 0);
	});

	it('refuses a have list over the cap', async () => {
		const have = Array.from({ length: 257 }, (_, i) => ({
			full: `@acme/h${i}`,
			version: '1.0.0',
			integrity: 'a'.repeat(64)
		}));
		const res = await call({ requirements: { '@acme/x': '*' }, have });
		assert.equal(res.status, 413);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['code'], 'payload-too-large');
		assert.match(String(body['message']), /have list over 256 entries/);
		assert.equal(fake.counters.roundTrips, 0);
	});

	it('keeps answering a request inside the caps', async () => {
		await seedPackage(fake.env, payload('@acme/inside', '1.0.0'));
		clearVersionIndex();
		const res = await call({ requirements: { '@acme/inside': '*' } });
		assert.equal(res.status, 200);
		const body = (await res.json()) as Record<string, unknown>;
		assert.deepEqual(Object.keys(body), ['resolved', 'base', 'levels']);
	});
});

describe('resolve have index', () => {
	it('compares the have list once instead of once per node', async () => {
		for (let i = 0; i < 5; i++) {
			await seedPackage(fake.env, payload(`@acme/have${i}`, '1.0.0'));
		}
		clearVersionIndex();
		resetResolveCounters();
		const have: HaveEntry[] = Array.from({ length: 20000 }, (_, i) => ({
			full: `@acme/other${i}`,
			version: '1.0.0',
			integrity: 'b'.repeat(64)
		}));
		const started = performance.now();
		const out = await resolveGraph(
			fake.env,
			{ '@acme/have0': '*', '@acme/have1': '*', '@acme/have2': '*', '@acme/have3': '*', '@acme/have4': '*' },
			have
		);
		const elapsed = performance.now() - started;
		assert.equal(Object.keys(out.resolved).length, 5);
		// The indexed lookup compares one entry per prune, not one pass over
		// the list per node.
		assert.ok(
			resolveCounters.haveComparisons <= 100,
			`have comparisons ${resolveCounters.haveComparisons}`
		);
		assert.ok(elapsed < 500, `resolve took ${elapsed} ms`);
	});

	it('still prunes a node the have list names', async () => {
		await seedPackage(fake.env, payload('@acme/pruned', '1.0.0'));
		clearVersionIndex();
		const out = await resolveGraph(fake.env, { '@acme/pruned': '*' }, [
			{ full: '@acme/pruned', version: '1.0.0', integrity: 'sha-@acme/pruned-1.0.0' }
		]);
		assert.deepEqual(out.levels[0].map((n) => n.full), ['@acme/pruned']);
		assert.deepEqual(out.levels[0][0].deps, []);
	});
});

describe('resolve subrequest budget', () => {
	it('refuses a closure that needs more subrequests than the plan allows', async () => {
		for (let i = 0; i < 63; i++) {
			await seedPackage(
				fake.env,
				payload(`@acme/chain${i}`, '1.0.0', {
					manifestJson:
						i === 0 ? '{}' : JSON.stringify({ deps: { [`@acme/chain${i - 1}`]: '*' } })
				})
			);
		}
		await seedPackage(fake.env, payload('@acme/chain63', '1.0.0', {
			manifestJson: JSON.stringify({ deps: { '@acme/chain62': '*' } })
		}));
		clearVersionIndex();
		resetResolveCounters();
		fake.reset();
		const res = await call({ requirements: { '@acme/chain63': '*' } });
		assert.equal(res.status, 422);
		const body = (await res.json()) as Record<string, unknown>;
		assert.equal(body['code'], 'budget-exceeded');
		assert.match(String(body['message']), /external subrequests over 50/);
		// The refusal came before the platform limit was reached.
		assert.ok(fake.counters.roundTrips <= RESOLVE_MAX_SUBREQUESTS);
		assert.ok(resolveCounters.subrequests <= RESOLVE_MAX_SUBREQUESTS);
	});

	it('spends a subrequest budget that a deployment can raise', async () => {
		await seedPackage(fake.env, payload('@acme/raised', '1.0.0'));
		clearVersionIndex();
		const out = await resolveGraph(
			{ ...fake.env, RESOLVE_MAX_SUBREQUESTS: '200' },
			{ '@acme/raised': '*' },
			[]
		);
		assert.deepEqual(Object.keys(out.resolved), ['@acme/raised']);
	});
});
