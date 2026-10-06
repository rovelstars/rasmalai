import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { r2Bucket, chunkKey, r2PutIfMissing, r2Get, r2Delete } from '../src/lib/server/r2.ts';
import {
	sampleProbability,
	freshQuotaState,
	effectiveUsage,
	shouldSample,
	quotaBudgetBytes,
	quotaGuardEnabled
} from '../src/lib/server/quota.ts';

function memBucket() {
	const store = new Map<string, Uint8Array>();
	let puts = 0;
	let heads = 0;
	return {
		puts: () => puts,
		heads: () => heads,
		async head(key: string) {
			heads++;
			return store.has(key) ? { key } : null;
		},
		async get(key: string) {
			const b = store.get(key);
			return b ? { arrayBuffer: async () => b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) as ArrayBuffer } : null;
		},
		async put(key: string, value: Uint8Array) {
			puts++;
			store.set(key, value);
			return { key };
		},
		async delete(key: string) {
			store.delete(key);
		}
	};
}

const H = 'ab'.padEnd(64, 'c');

describe('r2 key layout', () => {
	it('uses chunks/ab/cd/<hex> per spec', () => {
		assert.equal(chunkKey(H), `chunks/ab/cc/${H}`);
	});
});

describe('r2Bucket', () => {
	it('rejects missing and malformed bindings', () => {
		assert.equal(r2Bucket({}), null);
		assert.equal(r2Bucket({ CHUNKS: 'nope' }), null);
		assert.equal(r2Bucket({ CHUNKS: {} }), null);
	});

	it('accepts a structural bucket', () => {
		assert.notEqual(r2Bucket({ CHUNKS: memBucket() }), null);
	});
});

describe('r2 byte round-trip', () => {
	it('puts once and dedups on conflict', async () => {
		const b = memBucket();
		const bytes = new TextEncoder().encode('hello');
		assert.equal(await r2PutIfMissing(b, H, bytes), true);
		assert.equal(await r2PutIfMissing(b, H, bytes), false);
		assert.equal(b.puts(), 1);
		const back = await r2Get(b, H);
		assert.deepEqual(back, bytes);
	});

	it('deletes', async () => {
		const b = memBucket();
		await r2PutIfMissing(b, H, new Uint8Array([1]));
		await r2Delete(b, H);
		assert.equal(await r2Get(b, H), null);
	});
});

describe('pity sampler', () => {
	it('stays at base rate below soft pity', () => {
		assert.equal(sampleProbability(0), 0.001);
		assert.equal(sampleProbability(0.5), 0.001);
		assert.equal(sampleProbability(0.75), 0.001);
		assert.equal(sampleProbability(NaN), 0.001);
	});

	it('ramps quadratically to certain at hard pity', () => {
		const mid = sampleProbability(0.87);
		assert.ok(mid > 0.001 && mid < 1, `mid=${mid}`);
		assert.ok(sampleProbability(0.95) > mid);
		assert.equal(sampleProbability(0.99), 1);
		assert.equal(sampleProbability(1.5), 1);
	});

	it('cold state always samples, warm state rolls the pity', () => {
		const budget = 1000;
		const cold = freshQuotaState();
		assert.equal(shouldSample(cold, budget, 0.999), true);
		const warm = { sampled: 100, localBytes: 0 };
		assert.equal(shouldSample(warm, budget, 0.5), false);
		assert.equal(shouldSample(warm, budget, 0.0005), true);
		const hot = { sampled: 990, localBytes: 0 };
		assert.equal(shouldSample(hot, budget, 0.5), true);
	});

	it('effective usage adds local bytes over the sample', () => {
		assert.equal(effectiveUsage({ sampled: 500, localBytes: 100 }, 1000), 0.6);
		assert.equal(effectiveUsage({ sampled: null, localBytes: 100 }, 1000), 0.1);
		assert.equal(effectiveUsage(freshQuotaState(), 0), 0);
	});

	it('reads the budget override', () => {
		assert.equal(quotaBudgetBytes({}), 10 * 1024 * 1024 * 1024);
		assert.equal(quotaBudgetBytes({ QUOTA_R2_BYTES: '1024' }), 1024);
		assert.equal(quotaBudgetBytes({ QUOTA_R2_BYTES: 'junk' }), 10 * 1024 * 1024 * 1024);
	});

	it('kills the guard on QUOTA_GUARD=off', () => {
		assert.equal(quotaGuardEnabled({}), true);
		assert.equal(quotaGuardEnabled({ QUOTA_GUARD: 'off' }), false);
		assert.equal(quotaGuardEnabled({ QUOTA_GUARD: 'OFF' }), false);
		assert.equal(quotaGuardEnabled({ QUOTA_GUARD: 'on' }), true);
	});
});
