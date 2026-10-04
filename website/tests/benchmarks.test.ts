import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { validateSnapshot, unwrapEnvelope } from '../src/lib/server/benchmarks.js';

const row = {
	lang: 'rnx',
	mode: 'rel',
	label: 'rnx rel',
	runtime_ms: 10.5,
	peak_rss_mb: 3.1,
	build_ms: 42,
	build: 'rnx build --release',
	run: 'rnx run',
	artifact: true
};

const good = {
	benchmarks: {
		fib: { name: 'fib(35)', category: 'compute', description: 'd', results: [row] }
	},
	system: { cpu: 'x', os: 'Linux', date: '2026-10-04' },
	versions: { rnx: '0.1.0' }
};

describe('benchmark snapshot validation', () => {
	it('accepts a minimal valid snapshot', () => {
		assert.equal(validateSnapshot(good), null);
	});

	it('unwraps the /api/benchmarks envelope', () => {
		assert.equal(validateSnapshot({ snapshot: good, run: { id: 1 } }), null);
		const unwrapped = unwrapEnvelope({ snapshot: good });
		assert.equal('fib' in (unwrapped?.benchmarks ?? {}), true);
	});

	it('rejects empty and malformed snapshots', () => {
		assert.match(validateSnapshot({ benchmarks: {} }) ?? '', /no benchmarks/);
		assert.match(validateSnapshot(null) ?? '', /object/);
		assert.match(validateSnapshot({ benchmarks: { fib: {} } }) ?? '', /fib/);
	});

	it('rejects bad rows', () => {
		const badLabel = structuredClone(good);
		badLabel.benchmarks.fib.results = [{ ...row, label: 'wrong' }];
		assert.match(validateSnapshot(badLabel) ?? '', /bad label/);
		const badMode = structuredClone(good);
		badMode.benchmarks.fib.results = [{ ...row, mode: 'fast', label: 'rnx fast' }];
		assert.match(validateSnapshot(badMode) ?? '', /mode/);
		const zeroBuild = structuredClone(good);
		zeroBuild.benchmarks.fib.results = [{ ...row, build_ms: 0 }];
		assert.match(validateSnapshot(zeroBuild) ?? '', /zero build_ms/);
	});
});
