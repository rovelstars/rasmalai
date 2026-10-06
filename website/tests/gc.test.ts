import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { GC_DEFAULT_LIMIT, GC_GRACE_DAYS, GC_MAX_LIMIT, gcCutoffSec, parseGcLimit } from '../src/lib/server/gc.js';
import { isRequestId, newRequestId } from '../src/lib/server/requestId.js';

describe('gc cutoff', () => {
	it('holds chunks for the full 30d grace window', () => {
		const now = 1700000000;
		assert.equal(GC_GRACE_DAYS, 30);
		assert.equal(gcCutoffSec(now), now - 30 * 86400);
	});

	it('floors fractional inputs', () => {
		assert.equal(gcCutoffSec(100.9, 1), 100 - 86400);
	});
});

describe('parseGcLimit', () => {
	it('defaults when the value is missing or not a number', () => {
		assert.equal(parseGcLimit(undefined), GC_DEFAULT_LIMIT);
		assert.equal(parseGcLimit(null), GC_DEFAULT_LIMIT);
		assert.equal(parseGcLimit('abc'), GC_DEFAULT_LIMIT);
		assert.equal(parseGcLimit(Number.NaN), GC_DEFAULT_LIMIT);
	});

	it('accepts numbers and numeric strings, floored', () => {
		assert.equal(parseGcLimit(42.9), 42);
		assert.equal(parseGcLimit('50'), 50);
	});

	it('clamps to the 1..1000 sweep bound', () => {
		assert.equal(parseGcLimit(0), 1);
		assert.equal(parseGcLimit(-5), 1);
		assert.equal(parseGcLimit(5000), GC_MAX_LIMIT);
		assert.equal(GC_MAX_LIMIT, 1000);
	});
});

describe('request ids', () => {
	it('mints unique v4 ids', () => {
		const a = newRequestId();
		const b = newRequestId();
		assert.equal(isRequestId(a), true);
		assert.notEqual(a, b);
	});

	it('rejects non-ids', () => {
		assert.equal(isRequestId('nope'), false);
		assert.equal(isRequestId(''), false);
		assert.equal(isRequestId(undefined), false);
		assert.equal(isRequestId('550e8400-e29b-41d4-a716-446655440000'.toUpperCase()), false);
	});
});
