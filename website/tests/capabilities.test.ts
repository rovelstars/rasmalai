import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
	TIER_META,
	isTier,
	isHazardCapability,
	nodeLocation,
	validateManifest
} from '../src/lib/capabilities.js';

describe('tier presentation contract', () => {
	it('covers all four tiers with the specified pill classes', () => {
		assert.match(TIER_META.pure.pill, /bg-emerald-50/);
		assert.match(TIER_META.pure.pill, /text-emerald-700/);
		assert.match(TIER_META.pure.pill, /border-emerald-200/);
		assert.match(TIER_META.delegated.pill, /bg-amber-50/);
		assert.match(TIER_META.ambient.pill, /bg-orange-50/);
		assert.match(TIER_META.hazard.pill, /bg-red-50/);
		assert.equal(TIER_META.pure.label, 'PURE');
		assert.equal(TIER_META.delegated.label, 'DELEGATED');
		assert.equal(TIER_META.ambient.label, 'AMBIENT');
		assert.equal(TIER_META.hazard.label, 'HAZARD');
	});

	it('validates tier strings', () => {
		assert.equal(isTier('pure'), true);
		assert.equal(isTier('hazard'), true);
		assert.equal(isTier('bogus'), false);
		assert.equal(isTier(undefined), false);
	});

	it('flags hazard capabilities', () => {
		assert.equal(isHazardCapability('sys:exec:ffmpeg'), true);
		assert.equal(isHazardCapability('unsafe:ffi'), true);
		assert.equal(isHazardCapability('unsafe:raw_memory'), true);
		assert.equal(isHazardCapability('fs:delegated'), false);
		assert.equal(isHazardCapability('net:http:https://a.b/*'), false);
	});

	it('formats node locations as file:line:col', () => {
		assert.equal(
			nodeLocation({
				file: 'src/lib.rnx',
				line: 12,
				col: 5,
				symbol: 'loadData',
				expression_snippet: 'fn loadData(path: String): Int'
			}),
			'src/lib.rnx:12:5'
		);
	});
});

describe('manifest validation (rnx audit --json contract)', () => {
	const delegatedPayload = {
		name: 'delib',
		version: '0.3.0',
		tier: 'delegated',
		has_hazard: false,
		capabilities: ['fs:delegated'],
		traces: [
			{
				capability: 'fs:delegated',
				is_delegated: true,
				nodes: [
					{
						file: 'src/lib.rnx',
						line: 1,
						col: 1,
						symbol: 'loadData',
						expression_snippet: 'fn loadData(path: String): Int'
					},
					{
						file: 'src/lib.rnx',
						line: 2,
						col: 5,
						symbol: 'File.open',
						expression_snippet: 'File.open(path, FileMode.Read)'
					}
				]
			}
		],
		summary: 'delib v0.3.0: tier delegated, 1 capability'
	};

	it('accepts a realistic audit payload', () => {
		const m = validateManifest(delegatedPayload);
		assert.equal(m.name, 'delib');
		assert.equal(m.tier, 'delegated');
		assert.equal(m.traces?.length, 1);
		assert.equal(m.traces?.[0].nodes[0].symbol, 'loadData');
		assert.equal(m.traces?.[0].nodes.at(-1)?.symbol, 'File.open');
	});

	it('derives has_hazard from capabilities', () => {
		const m = validateManifest({
			name: 'tool',
			version: '1.0.0',
			tier: 'hazard',
			has_hazard: false,
			capabilities: ['sys:exec:oxipng']
		});
		assert.equal(m.has_hazard, true);
	});

	it('rejects malformed manifests', () => {
		assert.throws(() => validateManifest(null), /object/);
		assert.throws(() => validateManifest({ version: '1', tier: 'pure', capabilities: [] }), /name/);
		assert.throws(
			() => validateManifest({ name: 'x', version: '1', tier: 'bogus', capabilities: [] }),
			/tier/
		);
		assert.throws(
			() => validateManifest({ name: 'x', version: '1', tier: 'pure', capabilities: [42] }),
			/capabilities/
		);
		assert.throws(
			() =>
				validateManifest({
					name: 'x',
					version: '1',
					tier: 'pure',
					capabilities: [],
					traces: [{ capability: 'fs:delegated' }]
				}),
			/is_delegated/
		);
	});
});
