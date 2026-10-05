import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { pruneHit, type HaveEntry, type ResolveNode } from '../src/lib/server/db.js';
import { levelize } from '../src/lib/server/registry.js';

// resolveGraph needs Turso: db.ts imports the @libsql/client web flavor,
// which rejects file: URLs, so no temp-file DB works here. These tests pin
// the pure pieces instead: the have-pruning predicate and the exact
// levelize wiring resolveGraph uses (dep names mapped to full@version ids,
// levels mapped back to nodes).

function wireLevels(nodes: ResolveNode[], resolved: Record<string, string>): ResolveNode[][] {
	const order = levelize(
		nodes.map((n) => ({
			id: `${n.full}@${n.version}`,
			deps: n.deps.map((d) => {
				const v = resolved[d];
				return v ? `${d}@${v}` : d;
			})
		}))
	);
	const byId = new Map(nodes.map((n) => [`${n.full}@${n.version}`, n]));
	return order.map((ids) => ids.map((id) => byId.get(id)!));
}

describe('resolveGraph have-pruning', () => {
	it('prunes only on exact full/version/integrity match', () => {
		const have: HaveEntry[] = [{ full: '@acme/a', version: '1.0.0', integrity: 'abc' }];
		assert.equal(pruneHit(have, '@acme/a', '1.0.0', 'abc'), true);
		assert.equal(pruneHit(have, '@acme/a', '1.0.1', 'abc'), false);
		assert.equal(pruneHit(have, '@acme/a', '1.0.0', 'other'), false);
		assert.equal(pruneHit(have, '@acme/b', '1.0.0', 'abc'), false);
		assert.equal(pruneHit(have, '@acme/a', '1.0.0', ''), false);
		assert.equal(pruneHit([], '@acme/a', '1.0.0', 'abc'), false);
	});
});

describe('resolveGraph level shape', () => {
	it('levels a chain roots-first', () => {
		const nodes: ResolveNode[] = [
			{ full: 'a', version: '1.0.0', path: '', integrity: '', engineRange: '', deps: ['b'], yanked: false },
			{ full: 'b', version: '1.0.0', path: '', integrity: '', engineRange: '', deps: ['c'], yanked: false },
			{ full: 'c', version: '2.0.0', path: '', integrity: '', engineRange: '', deps: [], yanked: false }
		];
		const levels = wireLevels(nodes, { a: '1.0.0', b: '1.0.0', c: '2.0.0' });
		assert.deepEqual(
			levels.map((l) => l.map((n) => `${n.full}@${n.version}`)),
			[['a@1.0.0'], ['b@1.0.0'], ['c@2.0.0']]
		);
	});

	it('keeps a have-pruned node in its level with empty deps', () => {
		const nodes: ResolveNode[] = [
			{ full: 'a', version: '1.0.0', path: '', integrity: '', engineRange: '', deps: [], yanked: false },
			{ full: 'b', version: '1.0.0', path: '', integrity: '', engineRange: '', deps: [], yanked: false }
		];
		const levels = wireLevels(nodes, { a: '1.0.0', b: '1.0.0' });
		assert.equal(levels.length, 1);
		assert.deepEqual(
			levels[0].map((n) => n.full).sort(),
			['a', 'b']
		);
	});

	it('surfaces dependency cycles from the wiring', () => {
		const nodes: ResolveNode[] = [
			{ full: 'a', version: '1.0.0', path: '', integrity: '', engineRange: '', deps: ['b'], yanked: false },
			{ full: 'b', version: '1.0.0', path: '', integrity: '', engineRange: '', deps: ['a'], yanked: false }
		];
		assert.throws(() => wireLevels(nodes, { a: '1.0.0', b: '1.0.0' }), /cycle/);
	});
});
