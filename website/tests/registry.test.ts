import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
	parseSemver,
	compareSemver,
	satisfiesRange,
	maxSatisfying,
	selectLatestVersion,
	isValidRange,
	levelize
} from '../src/lib/server/registry.js';
import { buildTransferAuditEntries } from '../src/lib/server/db.js';

describe('semver', () => {
	it('parses strict versions', () => {
		assert.equal(parseSemver('1.2.3')?.major, 1);
		assert.equal(parseSemver('1.2.3-rc.1')?.prerelease, 'rc.1');
		assert.equal(parseSemver('1.2'), null);
		assert.equal(parseSemver('v1.2.3'), null);
	});

	it('orders stable above prerelease', () => {
		const a = parseSemver('1.0.0')!;
		const b = parseSemver('1.0.0-rc.1')!;
		assert.ok(compareSemver(a, b) > 0);
		assert.ok(compareSemver(parseSemver('2.0.0')!, a) > 0);
		assert.ok(compareSemver(parseSemver('1.0.10')!, parseSemver('1.0.9')!) > 0);
	});
});

describe('ranges', () => {
	it('matches caret, tilde, exact, gte, star', () => {
		assert.equal(satisfiesRange('1.4.3', '^1.0.0'), true);
		assert.equal(satisfiesRange('2.0.0', '^1.0.0'), false);
		assert.equal(satisfiesRange('0.2.5', '^0.2.0'), true);
		assert.equal(satisfiesRange('0.3.0', '^0.2.0'), false);
		assert.equal(satisfiesRange('1.2.9', '~1.2.0'), true);
		assert.equal(satisfiesRange('1.3.0', '~1.2.0'), false);
		assert.equal(satisfiesRange('1.2.3', '1.2.3'), true);
		assert.equal(satisfiesRange('1.2.4', '>=1.2.3'), true);
		assert.equal(satisfiesRange('9.9.9', '*'), true);
	});

	it('excludes prereleases unless the range names one (npm rule)', () => {
		assert.equal(satisfiesRange('2.0.0-rc.1', '^2.0.0'), false);
		assert.equal(satisfiesRange('2.0.0-rc.1', '*'), false);
		assert.equal(satisfiesRange('2.0.0-rc.1', 'latest'), false);
		assert.equal(satisfiesRange('2.0.0-rc.1', '2.0.0-rc.1'), true);
		assert.equal(satisfiesRange('2.0.0-rc.2', '^2.0.0-rc.1'), true);
		assert.equal(maxSatisfying(['1.0.0', '2.0.0-rc.1', '1.5.0'], '^1.0.0'), '1.5.0');
	});

	it('picks the max satisfying version', () => {
		assert.equal(maxSatisfying(['1.0.0', '1.4.3', '2.0.0'], '^1.0.0'), '1.4.3');
		assert.equal(maxSatisfying(['1.0.0'], '^2.0.0'), null);
	});

	it('validates the documented range grammar', () => {
		for (const r of ['*', 'latest', '', '1.2.3', '1.2.3-rc.1', '^1.0.0', '~0.2.0', '>=1.2.3', '>= 1.2.3']) {
			assert.equal(isValidRange(r), true, r);
		}
		for (const r of ['foo', '^', '~', '>=', '1.2', 'v1.2.3', '^foo', '>=bar', '<1.2.3', '1.2.3 || 2.0.0', '*.*']) {
			assert.equal(isValidRange(r), false, r);
		}
	});

	it('copies npm prerelease exclusion', () => {
		assert.equal(satisfiesRange('1.5.0-rc.1', '^1.0.0'), false);
		assert.equal(satisfiesRange('1.5.0-rc.1', '*'), false);
		assert.equal(satisfiesRange('1.5.0-rc.1', 'latest'), false);
		assert.equal(satisfiesRange('2.0.0-rc.2', '^2.0.0-rc.1'), true);
		assert.equal(satisfiesRange('2.0.1-rc.1', '^2.0.0-rc.1'), false);
		assert.equal(satisfiesRange('1.5.0-rc.1', '1.5.0-rc.1'), true);
		assert.equal(maxSatisfying(['1.4.3', '2.0.0-rc.1'], '*'), '1.4.3');
		assert.equal(maxSatisfying(['2.0.0-rc.1'], '*'), null);
	});
});

describe('transfer audit shape', () => {
	it('covers old and new names per version', () => {
		const rows = buildTransferAuditEntries('@old/pkg', '@new/pkg', ['1.0.0']);
		assert.deepEqual(
			rows.map((r) => [r.action, r.fullName, r.version]),
			[
				['transfer', '@old/pkg', '1.0.0'],
				['transfer', '@new/pkg', '1.0.0']
			]
		);
	});
});

describe('levelize', () => {
	it('orders by longest-path depth', () => {
		const levels = levelize([
			{ id: 'a', deps: ['b', 'c'] },
			{ id: 'b', deps: ['c'] },
			{ id: 'c', deps: [] }
		]);
		assert.deepEqual(levels, [['a'], ['b'], ['c']]);
	});

	it('rejects cycles', () => {
		assert.throws(() => levelize([
			{ id: 'a', deps: ['b'] },
			{ id: 'b', deps: ['a'] }
		]), /cycle/);
	});
});
