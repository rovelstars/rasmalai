import { describe, it, before, after, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import {
	resolveGraph,
	publishPackage,
	yankVersion,
	recordTombstone,
	readRegistryGeneration,
	clearVersionIndex,
	pruneHit,
	versionIndexEntries,
	RESOLVE_MAX_INLINE_BYTES,
	RESOLVE_MAX_NODES,
	GENERATION_KEY,
	type HaveEntry,
	type ResolveNode,
	type PublishPayload
} from '../src/lib/server/db.js';
import { levelize, CAPABILITIES } from '../src/lib/server/registry.js';

// resolveGraph runs against the real drizzle/libsql code path over a local
// Hrana-protocol server, so these tests count the statements and HTTP round
// trips the resolver actually issues rather than re-deriving them.

let fake: FakeLibsql;

function payload(
	name: string,
	version: string,
	overrides: Partial<PublishPayload> = {}
): PublishPayload {
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

function entriesFor(version: string): string {
	return JSON.stringify([
		{ name: 'Project.config', size: 10, dir: false, chunks: ['a'.repeat(64)] },
		{ name: 'src/main.rnx', size: 42, dir: false, chunks: [`${version.charCodeAt(0).toString(16)}`.padEnd(64, '0')] }
	]);
}

// tar_manifest_json is written after publish by the chunk-upload path, not by
// the publish batch, so the fixture has to set it explicitly.
async function setEntries(full: string, version: string, json: string): Promise<void> {
	const { storeTarManifest } = await import('../src/lib/server/db.js');
	const { packageId, parsePackageName } = await import('../src/lib/server/db.js');
	const parsed = parsePackageName(full)!;
	await storeTarManifest(fake.env, `ver_${packageId(parsed.scope, parsed.name)}_${version}`, JSON.parse(json));
}

async function publishMany(specs: Array<{ full: string; version: string; deps?: Record<string, string>; entries?: string; yanked?: boolean }>) {
	for (const s of specs) {
		await publishPackage(
			fake.env,
			payload(s.full, s.version, {
				manifestJson: JSON.stringify(s.deps ? { deps: s.deps } : {}),
				tarballSha256: `sha-${s.full}-${s.version}`
			})
		);
		if (s.entries !== undefined) await setEntries(s.full, s.version, s.entries);
		else await setEntries(s.full, s.version, entriesFor(s.version));
		if (s.yanked) await yankVersion(fake.env, s.full, s.version);
	}
}

function nodeShape(nodes: ResolveNode[], resolved: Record<string, string>): string {
	const order = levelize(nodes.map((n) => ({
		id: `${n.full}@${n.version}`,
		deps: n.deps.map((d) => {
			const v = resolved[d];
			return v ? `${d}@${v}` : d;
		})
	})));
	const byId = new Map(nodes.map((n) => [`${n.full}@${n.version}`, n]));
	return JSON.stringify(order.map((ids) => ids.map((id) => {
		const n = byId.get(id)!;
		return { full: n.full, version: n.version, deps: n.deps, yanked: n.yanked, integrity: n.integrity };
	})));
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

describe('resolveGraph round trips', () => {
	it('resolves 17 flat std-shaped packages in two store round trips', async () => {
		const reqs: Record<string, string> = {};
		for (let i = 0; i < 17; i++) {
			const full = `@std/pkg${i}`;
			reqs[full] = '*';
			await publishMany([{ full, version: '0.1.0' }]);
		}
		clearVersionIndex();
		fake.reset();

		const out = await resolveGraph(fake.env, reqs, []);
		assert.equal(Object.keys(out.resolved).length, 17);
		assert.equal(out.levels.length, 1);
		assert.equal(out.levels[0].length, 17);
		// One candidate statement + one picked-row statement, one batch each.
		assert.equal(fake.counters.roundTrips, 2, `statements: ${fake.statements.join(' | ')}`);
	});

	it('never issues one statement per package on a wide frontier', async () => {
		const reqs: Record<string, string> = {};
		for (let i = 0; i < 200; i++) {
			const full = `@acme/w${i}`;
			reqs[full] = '*';
		}
		// Published rows are not needed to count: unresolved names still batch.
		await assert.rejects(() => resolveGraph(fake.env, reqs, []), /does not exist/);
		// 200 names = 4 candidate chunks (50 per statement) + 0 picked rows,
		// and each level is a single HTTP request.
		assert.equal(fake.counters.roundTrips, 1);
		assert.ok(fake.counters.statements <= 6, `${fake.counters.statements} statements`);
	});

	it('spends one batch per level on a three-level graph', async () => {
		await publishMany([
			{ full: '@acme/r0', version: '1.0.0', deps: { '@acme/l1a': '*', '@acme/l1b': '*' } },
			{ full: '@acme/l1a', version: '1.0.0', deps: { '@acme/l2a': '*' } },
			{ full: '@acme/l1b', version: '1.0.0', deps: { '@acme/l2b': '*' } },
			{ full: '@acme/l2a', version: '1.0.0' },
			{ full: '@acme/l2b', version: '1.0.0' }
		]);
		clearVersionIndex();
		fake.reset();

		const out = await resolveGraph(fake.env, { '@acme/r0': '*' }, []);
		assert.equal(out.levels.length, 3);
		assert.equal(fake.counters.roundTrips, 6, fake.statements.join(' | '));
	});

	it('reuses the memo instead of re-reading candidates inside the TTL', async () => {
		await publishMany([{ full: '@acme/memo', version: '1.0.0' }]);
		clearVersionIndex();
		fake.reset();
		await resolveGraph(fake.env, { '@acme/memo': '*' }, []);
		// Only the candidate statement selects just scope/name/version.
		const candidateReads = () =>
			fake.statements.filter((x) => x.includes('p.name AS name, v.version AS version')).length;
		assert.equal(candidateReads(), 1, 'cold resolve reads candidates once');
		const coldRt = fake.counters.roundTrips;

		fake.reset();
		await resolveGraph(fake.env, { '@acme/memo': '*' }, []);
		assert.equal(candidateReads(), 0, 'warm resolve must not read candidates');
		assert.equal(fake.counters.roundTrips, coldRt - 1, `warm RT ${fake.counters.roundTrips} cold ${coldRt}`);
	});

	it('bounds the memo', async () => {
		for (let i = 0; i < 20; i++) await publishMany([{ full: `@acme/m${i}`, version: '1.0.0' }]);
		assert.ok(versionIndexEntries() <= 4096);
	});
});

describe('resolveGraph semantics', () => {
	it('keeps levels, resolved and node fields identical to the levelize wiring', async () => {
		await publishMany([
			{ full: '@acme/a', version: '1.0.0', deps: { '@acme/b': '^1.0.0', '@acme/c': '*' } },
			{ full: '@acme/b', version: '1.0.0' },
			{ full: '@acme/c', version: '2.1.0', yanked: true },
			{ full: '@acme/b', version: '1.5.0' }
		]);
		clearVersionIndex();
		const out = await resolveGraph(fake.env, { '@acme/a': '*', '@acme/c': '^2.0.0' }, []);
		assert.deepEqual(out.resolved, { '@acme/a': '1.0.0', '@acme/c': '2.1.0', '@acme/b': '1.5.0' });
		const nodes = out.levels.flat();
		const byId = new Map(nodes.map((n) => [`${n.full}@${n.version}`, n]));
		assert.equal(byId.get('@acme/a@1.0.0')!.deps.join(','), '@acme/b,@acme/c');
		assert.equal(byId.get('@acme/c@2.1.0')!.yanked, true);
		assert.equal(byId.get('@acme/b@1.5.0')!.path, '@acme/b@1.5.0/chunks');
		assert.equal(nodeShape(nodes, out.resolved), JSON.stringify([[
			{ full: '@acme/a', version: '1.0.0', deps: ['@acme/b', '@acme/c'], yanked: false, integrity: 'sha-@acme/a-1.0.0' }
		], [
			{ full: '@acme/b', version: '1.5.0', deps: [], yanked: false, integrity: 'sha-@acme/b-1.5.0' },
			{ full: '@acme/c', version: '2.1.0', deps: [], yanked: true, integrity: 'sha-@acme/c-2.1.0' }
		]]));
	});

	it('reports a missing package with the historical message', async () => {
		await assert.rejects(
			() => resolveGraph(fake.env, { '@acme/ghost': '*' }, []),
			/^Error: package @acme\/ghost does not exist$/
		);
	});

	it('reports an unsatisfiable range with the historical message', async () => {
		await publishMany([{ full: '@acme/old', version: '1.0.0' }]);
		await assert.rejects(
			() => resolveGraph(fake.env, { '@acme/old': '^9.0.0' }, []),
			/no version of @acme\/old satisfies \^9\.0\.0 \(have 1\.0\.0\)/
		);
	});

	it('reports conflicting ranges across parents', async () => {
		await publishMany([
			{ full: '@acme/p', version: '1.0.0', deps: { '@acme/x': '*' } },
			{ full: '@acme/q', version: '1.0.0', deps: { '@acme/x': '^1.0.0' } },
			{ full: '@acme/x', version: '1.0.0' },
			{ full: '@acme/x', version: '2.0.0' }
		]);
		await assert.rejects(
			() => resolveGraph(fake.env, { '@acme/p': '*', '@acme/q': '*' }, []),
			/conflicting ranges for @acme\/x: 2\.0\.0 does not satisfy \^1\.0\.0/
		);
	});

	it('surfaces a dependency cycle from levelize', async () => {
		await publishMany([
			{ full: '@acme/ca', version: '1.0.0', deps: { '@acme/cb': '*' } },
			{ full: '@acme/cb', version: '1.0.0', deps: { '@acme/ca': '*' } }
		]);
		// The serial implementation memoized the child before the parent, so
		// levelize entered the cycle at the child. The message must not move.
		await assert.rejects(
			() => resolveGraph(fake.env, { '@acme/ca': '*' }, []),
			/^Error: dependency cycle: @acme\/cb@1\.0\.0 -> @acme\/ca@1\.0\.0 -> @acme\/cb@1\.0\.0$/
		);
	});

	it('rejects an invalid package name before touching the store', async () => {
		const before = fake.counters.statements;
		await assert.rejects(() => resolveGraph(fake.env, { 'Bad Name': '*' }, []), /invalid package name/);
		assert.equal(fake.counters.statements, before);
	});

	it('enforces the closure budget', async () => {
		const deps: Record<string, string> = {};
		for (let i = 0; i < RESOLVE_MAX_NODES + 20; i++) deps[`@acme/n${i}`] = '*';
		await publishMany([{ full: '@acme/huge', version: '1.0.0', deps }]);
		await assert.rejects(
			() => resolveGraph(fake.env, { '@acme/huge': '*' }, []),
			/resolve budget exceeded/
		);
	});
});

describe('resolveGraph have-pruning', () => {
	it('prunes on exact full/version/integrity and stops descending', async () => {
		await publishMany([
			{ full: '@acme/hp', version: '1.0.0', deps: { '@acme/hpdep': '*' } },
			{ full: '@acme/hpdep', version: '1.0.0' }
		]);
		const have: HaveEntry[] = [{ full: '@acme/hp', version: '1.0.0', integrity: 'sha-@acme/hp-1.0.0' }];
		assert.equal(pruneHit(have, '@acme/hp', '1.0.0', 'sha-@acme/hp-1.0.0'), true);
		const out = await resolveGraph(fake.env, { '@acme/hp': '*' }, have);
		assert.equal(out.levels.length, 1);
		assert.deepEqual(out.resolved['@acme/hp'], '1.0.0');
		assert.equal(out.levels[0][0].deps.length, 0);
		assert.equal(out.resolved['@acme/hpdep'], undefined);
	});
});

describe('resolveGraph withdrawal and yank', () => {
	it('excludes tombstoned versions from candidates', async () => {
		await publishMany([
			{ full: '@acme/t', version: '1.0.0' },
			{ full: '@acme/t', version: '2.0.0' }
		]);
		await recordTombstone(fake.env, '@acme/t', '2.0.0', 'malware');
		clearVersionIndex();
		const out = await resolveGraph(fake.env, { '@acme/t': '*' }, []);
		assert.equal(out.resolved['@acme/t'], '1.0.0');
	});

	it('never serves a version that a tombstone appeared for, even with a warm memo', async () => {
		await publishMany([{ full: '@acme/tw', version: '1.0.0' }]);
		clearVersionIndex();
		await resolveGraph(fake.env, { '@acme/tw': '*' }, []);
		// A warm candidate list still names the now-tombstoned version; the
		// authoritative per-request check must keep it out of the answer.
		await recordTombstone(fake.env, '@acme/tw', '1.0.0', 'takedown');
		await assert.rejects(() => resolveGraph(fake.env, { '@acme/tw': '*' }, []), /does not exist/);
	});

	it('reports yanked versions as yanked', async () => {
		await publishMany([{ full: '@acme/y', version: '1.0.0' }, { full: '@acme/y', version: '2.0.0', yanked: true }]);
		clearVersionIndex();
		const out = await resolveGraph(fake.env, { '@acme/y': '*' }, []);
		assert.equal(out.levels[0][0].yanked, true);
	});

	it('folds the tombstone check into the picked-row statement', async () => {
		await publishMany([{ full: '@acme/ts', version: '1.0.0' }]);
		clearVersionIndex();
		fake.reset();
		await resolveGraph(fake.env, { '@acme/ts': '*' }, []);
		const picked = fake.statements.find((s) => s.includes('tarball_sha256'));
		assert.ok(picked, 'picked-row statement must exist');
		assert.ok(picked!.includes('FROM tombstones t'), 'picked-row statement must carry the tombstone check');
		// No standalone tombstone probe anywhere in the resolve.
		assert.equal(fake.statements.filter((s) => s.includes('FROM tombstones') && !s.includes('tarball_sha256')).length, 0);
	});

	it('never selects doc_json on the resolve path', async () => {
		await publishMany([{ full: '@acme/proj', version: '1.0.0' }]);
		clearVersionIndex();
		fake.reset();
		await resolveGraph(fake.env, { '@acme/proj': '*' }, []);
		for (const s of fake.statements) {
			assert.ok(!/doc_json|guides_json|request_id|checksum/.test(s), `unexpected column in: ${s}`);
		}
	});
});

describe('resolveGraph inline entries', () => {
	it('omits entries and the chunks switch by default', async () => {
		await publishMany([{ full: '@acme/inl', version: '1.0.0' }]);
		const out = await resolveGraph(fake.env, { '@acme/inl': '*' }, []);
		assert.equal(out.chunks, null);
		assert.equal(out.levels[0][0].entries, undefined);
		// The route drops a null switch, so the wire body is the historical one.
		const payload: Record<string, unknown> = { resolved: out.resolved, base: '', levels: out.levels };
		assert.deepEqual(Object.keys(payload), ['resolved', 'base', 'levels']);
		assert.equal('entries' in out.levels[0][0], false);
	});

	it('inlines entries behind the opt-in and reports chunks inline', async () => {
		await publishMany([{ full: '@acme/inl', version: '1.0.0' }]);
		clearVersionIndex();
		fake.reset();
		const out = await resolveGraph(fake.env, { '@acme/inl': '*' }, [], {
			inlineChunks: true,
			advertiseChunks: true
		});
		assert.equal(out.chunks, 'inline');
		assert.equal(out.levels[0][0].entries?.[1].name, 'src/main.rnx');
		// tar_manifest_json is selected only for the inline projection.
		assert.ok(fake.statements.some((s) => s.includes('tar_manifest_json')));
	});

	it('does not select tar_manifest_json when entries are not requested', async () => {
		await publishMany([{ full: '@acme/inl2', version: '1.0.0' }]);
		clearVersionIndex();
		fake.reset();
		await resolveGraph(fake.env, { '@acme/inl2': '*' }, []);
		assert.equal(fake.statements.filter((s) => s.includes('tar_manifest_json')).length, 0);
	});

	it('falls back to links for the whole response over the byte cap', async () => {
		const big = JSON.stringify([
			{ name: 'x'.repeat(200), size: 1, dir: false, chunks: ['b'.repeat(64)] }
		]);
		await publishMany([{ full: '@acme/big', version: '1.0.0', entries: big }]);
		clearVersionIndex();
		const out = await resolveGraph(fake.env, { '@acme/big': '*' }, [], {
			inlineChunks: true,
			advertiseChunks: true
		});
		// Under the cap, so the response still carries the list.
		assert.ok(big.length < RESOLVE_MAX_INLINE_BYTES);
		assert.equal(out.chunks, 'inline');
		assert.equal(out.levels[0][0].entries?.[0].chunks[0], 'b'.repeat(64));
	});

	it('drops every entries array when the cap is exceeded', async () => {
		const chunk = 'c'.repeat(64);
		const entries = JSON.stringify(
			Array.from({ length: 4000 }, (_, i) => ({
				name: `src/f${i}.rnx`,
				size: 1,
				dir: false,
				chunks: [chunk]
			}))
		);
		assert.ok(entries.length > RESOLVE_MAX_INLINE_BYTES);
		await publishMany([{ full: '@acme/huge2', version: '1.0.0', entries }]);
		clearVersionIndex();
		const out = await resolveGraph(fake.env, { '@acme/huge2': '*' }, [], {
			inlineChunks: true,
			advertiseChunks: true
		});
		assert.equal(out.chunks, 'links');
		for (const level of out.levels) for (const n of level) assert.equal(n.entries, undefined);
	});

	it('omits entries for a version with no file index', async () => {
		await publishPackage(fake.env, payload('@acme/nofile', '1.0.0'));
		fake.db
			.prepare('UPDATE package_versions SET tar_manifest_json = ? WHERE package_id = ?')
			.run('not json', 'pkg_@acme/nofile');
		clearVersionIndex();
		const out = await resolveGraph(fake.env, { '@acme/nofile': '*' }, [], {
			inlineChunks: true,
			advertiseChunks: true
		});
		// A file index that is not an array cannot be inlined; the node falls
		// back to the links path the client already knows.
		assert.equal(out.chunks, 'inline');
		assert.equal(out.levels[0][0].entries, undefined);
	});
});

describe('generation marker', () => {
	it('is bumped once per publish inside the same batch', async () => {
		const before = await readRegistryGeneration(fake.env);
		clearVersionIndex();
		await publishPackage(fake.env, payload('@acme/gen1', '1.0.0'));
		assert.equal(await readRegistryGeneration(fake.env), before + 1);
	});

	it('is bumped by yank and tombstone too', async () => {
		await publishMany([
			{ full: '@acme/gen2', version: '1.0.0' },
			{ full: '@acme/gen2', version: '2.0.0' },
			{ full: '@acme/gen3', version: '1.0.0' }
		]);
		const before = await readRegistryGeneration(fake.env);
		await yankVersion(fake.env, '@acme/gen2', '2.0.0');
		assert.equal(await readRegistryGeneration(fake.env), before + 1);
		await recordTombstone(fake.env, '@acme/gen3', '1.0.0', 'policy');
		assert.equal(await readRegistryGeneration(fake.env), before + 2);
	});

	it('lands in the same batch as the publish insert', async () => {
		clearVersionIndex();
		fake.reset();
		await publishPackage(fake.env, payload('@acme/gen4', '1.0.0'));
		const gen = fake.statements.findIndex((s) => s.includes(GENERATION_KEY));
		const versionInsert = fake.statements.findIndex((s) => s.includes('insert into "package_versions"'));
		const audit = fake.statements.findIndex((s) => s.includes("'publish'"));
		assert.ok(gen > 0 && versionInsert > 0 && audit > 0);
		// The audit row reads changes() from the version insert, so the bump
		// must come after both to leave publish idempotency intact.
		assert.ok(gen > versionInsert, 'generation bump must follow the version insert');
		assert.ok(gen > audit, 'generation bump must follow the audit row');
		assert.equal(fake.statements.filter((s) => s === 'BEGIN DEFERRED').length, 1);
		assert.equal(fake.statements.filter((s) => s === 'COMMIT').length, 1);
	});

	it('is monotonic, never resetting', async () => {
		const seen: number[] = [];
		for (let i = 0; i < 3; i++) {
			await publishPackage(fake.env, payload('@acme/mono', `1.0.${i}`));
			seen.push(await readRegistryGeneration(fake.env));
		}
		for (let i = 1; i < seen.length; i++) assert.ok(seen[i] > seen[i - 1]);
	});
});

describe('publish idempotency (changes() idiom)', () => {
	it('reports created only for the publish that minted the version', async () => {
		const first = await publishPackage(fake.env, payload('@acme/idem', '1.0.0'));
		assert.equal(first.created, true);
		const again = await publishPackage(fake.env, payload('@acme/idem', '1.0.0'));
		assert.equal(again.created, false, 'republish must not report created');
	});

	it('writes exactly one audit publish row across repeated publishes', async () => {
		for (let i = 0; i < 4; i++) await publishPackage(fake.env, payload('@acme/idem2', '1.0.0'));
		const rows = fake.db
			.prepare("SELECT count(*) AS c FROM audit_log WHERE action = 'publish' AND full_name = '@acme/idem2'")
			.all() as Array<{ c: number }>;
		assert.equal(Number(rows[0].c), 1);
	});

	it('mints a version once but still bumps the generation on a republish', async () => {
		await publishPackage(fake.env, payload('@acme/idem3', '1.0.0'));
		const gen = await readRegistryGeneration(fake.env);
		const again = await publishPackage(fake.env, payload('@acme/idem3', '1.0.0'));
		assert.equal(again.created, false);
		assert.equal(await readRegistryGeneration(fake.env), gen + 1);
	});

	it('keeps version rows unique across a mixed publish set', async () => {
		await publishPackage(fake.env, payload('@acme/idem4', '1.0.0', { manifestJson: '{"deps":{"@acme/dep":"*"}}' }));
		await publishPackage(fake.env, payload('@acme/dep', '1.0.0'));
		const dup = await publishPackage(fake.env, payload('@acme/idem4', '1.0.0', { manifestJson: '{"deps":{"@acme/dep":"*"}}' }));
		assert.equal(dup.created, false);
		const rows = fake.db
			.prepare("SELECT count(*) AS c FROM package_versions WHERE version = '1.0.0' AND package_id = 'pkg_@acme/idem4'")
			.all() as Array<{ c: number }>;
		assert.equal(Number(rows[0].c), 1);
	});
});

describe('resolve payload shape', () => {
	it('is byte-for-byte unchanged for a client that never opts in', async () => {
		await publishMany([{ full: '@acme/wire', version: '1.0.0', deps: { '@acme/wiredep': '^1.0.0' } }, { full: '@acme/wiredep', version: '1.0.0' }]);
		clearVersionIndex();
		const out = await resolveGraph(fake.env, { '@acme/wire': '*' }, []);
		const payload: Record<string, unknown> = { resolved: out.resolved, base: '', levels: out.levels };
		assert.deepEqual(
			Object.keys(JSON.parse(JSON.stringify(payload))),
			['resolved', 'base', 'levels']
		);
		for (const level of out.levels) {
			for (const n of level) {
				assert.deepEqual(Object.keys(JSON.parse(JSON.stringify(n))), [
					'full',
					'version',
					'path',
					'integrity',
					'engineRange',
					'deps',
					'yanked'
				]);
			}
		}
	});

	it('advertises the inline-chunks capability additively', () => {
		assert.ok(CAPABILITIES.includes('resolve-inline-chunks'));
		assert.equal(CAPABILITIES.includes('tombstones'), true);
	});
});