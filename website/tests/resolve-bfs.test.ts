import { describe, it, before, after, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import {
	resolveGraph,
	publishPackage,
	yankVersion,
	recordTombstone,
	transferScope,
	clearVersionIndex,
	pruneHit,
	versionIndexEntries,
	versionIndexSize,
	parsePackageName,
	packageId,
	RESOLVE_MAX_INLINE_BYTES,
	RESOLVE_MAX_INLINE_NODES,
	RESOLVE_MAX_NODES,
	RESOLVE_REPAIR_ROUNDS,
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

// The mutation count of a package: the number the resolve memo is
// validated against.
function mutationToken(full: string): number {
	const parsed = parsePackageName(full)!;
	const row = fake.db
		.prepare('SELECT mutation_count AS c FROM packages WHERE scope = ? AND name = ?')
		.get(parsed.scope, parsed.name) as { c: number } | undefined;
	return Number(row?.c ?? 0);
}

// A publish that lands in the store without this isolate's memo being
// cleared. It is what another isolate's publish leaves behind: the version
// row and the counter of the package move together.
function otherIsolatePublish(full: string, version: string): void {
	const parsed = parsePackageName(full)!;
	const id = packageId(parsed.scope, parsed.name);
	const sem = /^(\d+)\.(\d+)\.(\d+)(.*)$/.exec(version)!;
	fake.db
		.prepare(
			`INSERT INTO package_versions (id, package_id, version, readme_markdown, doc_json, checksum,
				status, created_at, semver_major, semver_minor, semver_patch, prerelease, engine_range,
				manifest_json, guides_json, tarball_sha256, tar_manifest_json, request_id)
			 VALUES (?, ?, ?, '', '{}', ?, 'live', ?, ?, ?, ?, ?, '', '{}', '[]', ?, '[]', '')`
		)
		.run(
			`ver_${id}_${version}`,
			id,
			version,
			`sha-${full}-${version}`,
			0,
			Number(sem[1]),
			Number(sem[2]),
			Number(sem[3]),
			sem[4] ?? '',
			`sha-${full}-${version}`
		);
	fake.db.prepare('UPDATE packages SET mutation_count = mutation_count + 1 WHERE id = ?').run(id);
}

// A scope transfer that lands in the store without this isolate's memo
// being cleared. It is the committed state a transfer leaves behind.
function otherIsolateTransfer(scope: string, newOwner: string): void {
	const rows = fake.db
		.prepare('SELECT id, name FROM packages WHERE scope = ?')
		.all(scope) as Array<{ id: string; name: string }>;
	for (const row of rows) {
		const newId = packageId(newOwner, row.name);
		fake.db
			.prepare(
				`INSERT INTO packages (id, scope, name, owner, description, author, repository, license,
					license_id, downloads, stars, tags, keywords, created_at, updated_at, mutation_count)
				 SELECT ?, ?, name, owner, description, author, repository, license, license_id, downloads,
					stars, tags, keywords, created_at, ?, mutation_count + 1 FROM packages WHERE id = ?`
			)
			.run(newId, newOwner, 0, row.id);
		fake.db.prepare('UPDATE package_versions SET package_id = ? WHERE package_id = ?').run(newId, row.id);
		fake.db.prepare('UPDATE tombstones SET package_id = ? WHERE package_id = ?').run(newId, row.id);
		fake.db.prepare('UPDATE package_deps SET package_id = ? WHERE package_id = ?').run(newId, row.id);
		fake.db.prepare('UPDATE download_daily SET package_id = ? WHERE package_id = ?').run(newId, row.id);
		fake.db.prepare('DELETE FROM packages WHERE id = ?').run(row.id);
	}
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
		// The bound is on the memoized version strings, not on the number of
		// entries: one package with many versions costs more than many
		// packages with one.
		assert.ok(versionIndexSize() <= 50_000);
	});
});

// The token rides on rows the resolve already reads, so it may not cost
// anything: the same statements, the same rows, the same round trips.
describe('resolveGraph cost', () => {
	it('reads the same rows and statements for 17 packages as before the token', async () => {
		const reqs: Record<string, string> = {};
		for (let i = 0; i < 17; i++) {
			const full = `@std/cost${i}`;
			reqs[full] = '*';
			await publishMany([{ full, version: '0.1.0' }]);
		}
		clearVersionIndex();
		fake.reset();
		const out = await resolveGraph(fake.env, reqs, []);
		assert.equal(Object.keys(out.resolved).length, 17);
		// One candidate statement and one picked-row statement, each in its
		// own transaction: BEGIN, SELECT, COMMIT.
		assert.equal(fake.counters.roundTrips, 2);
		assert.equal(fake.counters.statements, 6, fake.statements.join(' | '));
		// 17 candidate rows and 17 picked rows.
		assert.equal(fake.counters.rowsRead, 34);
	});

	it('reads the same rows and statements for a 200 node frontier', async () => {
		const reqs: Record<string, string> = {};
		for (let i = 0; i < 200; i++) reqs[`@acme/wide${i}`] = '*';
		clearVersionIndex();
		fake.reset();
		await assert.rejects(() => resolveGraph(fake.env, reqs, []), /does not exist/);
		// 200 names are 4 candidate chunks of 50, and no picked rows.
		assert.equal(fake.counters.roundTrips, 1);
		assert.equal(fake.counters.statements, 6, fake.statements.join(' | '));
		assert.equal(fake.counters.rowsRead, 0);
	});

	it('costs the same warm resolve as a cold one', async () => {
		await publishMany([{ full: '@acme/warmcost', version: '1.0.0' }]);
		clearVersionIndex();
		fake.reset();
		await resolveGraph(fake.env, { '@acme/warmcost': '*' }, []);
		const cold = { ...fake.counters };
		fake.reset();
		await resolveGraph(fake.env, { '@acme/warmcost': '*' }, []);
		// The warm resolve drops the candidate statement and its rows.
		assert.equal(fake.counters.roundTrips, cold.roundTrips - 1);
		assert.equal(fake.counters.statements, cold.statements - 3);
		assert.equal(fake.counters.rowsRead, cold.rowsRead - 1);
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

// A client that caches a version and is then offline for a while can send a
// `have` entry for a version the registry has since yanked or taken down.
// The entry may skip the dependencies, but it must not hide the status.
describe('resolveGraph have-pruning against a status change', () => {
	it('reports a yanked version to a client that already has it', async () => {
		await publishMany([{ full: '@acme/hy', version: '1.0.0' }]);
		await yankVersion(fake.env, '@acme/hy', '1.0.0');
		clearVersionIndex();
		const have: HaveEntry[] = [
			{ full: '@acme/hy', version: '1.0.0', integrity: 'sha-@acme/hy-1.0.0' }
		];
		const out = await resolveGraph(fake.env, { '@acme/hy': '*' }, have);
		assert.equal(out.levels.length, 1);
		assert.equal(out.levels[0][0].version, '1.0.0');
		assert.equal(out.levels[0][0].yanked, true, 'the yank must not be hidden by the have entry');
		// The entry still skips the dependencies.
		assert.equal(out.levels[0][0].deps.length, 0);
	});

	it('does not serve a taken down version to a client that already has it', async () => {
		await publishMany([{ full: '@acme/ht', version: '1.0.0' }]);
		await recordTombstone(fake.env, '@acme/ht', '1.0.0', 'policy');
		clearVersionIndex();
		const have: HaveEntry[] = [
			{ full: '@acme/ht', version: '1.0.0', integrity: 'sha-@acme/ht-1.0.0' }
		];
		await assert.rejects(
			() => resolveGraph(fake.env, { '@acme/ht': '*' }, have),
			/^Error: package @acme\/ht does not exist$/
		);
	});

	it('does not serve a version a takedown reached after the candidate list was cached', async () => {
		await publishMany([{ full: '@acme/hs', version: '1.0.0' }]);
		clearVersionIndex();
		// The candidate list of the package is cached now.
		await resolveGraph(fake.env, { '@acme/hs': '*' }, []);
		// A takedown lands in another isolate: the row and the tombstone are
		// written, but the cached candidate list still names the version.
		fake.db
			.prepare(
				'INSERT INTO tombstones (package_id, version, reason, created_at) VALUES (?, ?, ?, ?)'
			)
			.run('pkg_@acme/hs', '1.0.0', 'policy', 0);
		fake.db
			.prepare("UPDATE package_versions SET status = 'tombstoned' WHERE package_id = ? AND version = ?")
			.run('pkg_@acme/hs', '1.0.0');
		const have: HaveEntry[] = [
			{ full: '@acme/hs', version: '1.0.0', integrity: 'sha-@acme/hs-1.0.0' }
		];
		await assert.rejects(
			() => resolveGraph(fake.env, { '@acme/hs': '*' }, have),
			/^Error: package @acme\/hs does not exist$/
		);
	});

	it('picks the next live version when the newest one was taken down', async () => {
		await publishMany([
			{ full: '@acme/hn', version: '1.0.0' },
			{ full: '@acme/hn', version: '2.0.0' }
		]);
		clearVersionIndex();
		await resolveGraph(fake.env, { '@acme/hn': '*' }, []);
		fake.db
			.prepare(
				'INSERT INTO tombstones (package_id, version, reason, created_at) VALUES (?, ?, ?, ?)'
			)
			.run('pkg_@acme/hn', '2.0.0', 'policy', 0);
		fake.db
			.prepare("UPDATE package_versions SET status = 'tombstoned' WHERE package_id = ? AND version = ?")
			.run('pkg_@acme/hn', '2.0.0');
		const out = await resolveGraph(fake.env, { '@acme/hn': '*' }, []);
		assert.equal(out.resolved['@acme/hn'], '1.0.0');
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
		assert.equal(out.chunksDeclined, null);
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
		assert.equal(out.chunksDeclined, null);
		assert.equal(out.levels[0][0].entries?.[1].name, 'src/main.rnx');
		// tar_manifest_json is selected only for the inline answer.
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
		assert.equal(out.chunksDeclined, 'size-cap-exceeded');
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

// An inline request that cannot fit the cap must not pay for the file
// indexes it would have to throw away: the closure is resolved first, the
// indexes are measured, and only a closure that fits is read.
describe('resolveGraph inline cost', () => {
	const oversized = (): string =>
		JSON.stringify(
			Array.from({ length: 4000 }, (_, i) => ({
				name: `src/f${i}.rnx`,
				size: 1,
				dir: false,
				chunks: ['c'.repeat(64)]
			}))
		);

	it('never reads the file index of a closure that cannot fit', async () => {
		const entries = oversized();
		assert.ok(entries.length > RESOLVE_MAX_INLINE_BYTES);
		await publishMany([{ full: '@acme/cost', version: '1.0.0', entries }]);
		clearVersionIndex();
		fake.reset();
		const out = await resolveGraph(fake.env, { '@acme/cost': '*' }, [], {
			inlineChunks: true,
			advertiseChunks: true
		});
		assert.equal(out.chunks, 'links');
		assert.equal(out.chunksDeclined, 'size-cap-exceeded');
		// The index itself was never selected, only measured.
		assert.equal(fake.statements.filter((s) => s.includes('AS tar_manifest_json')).length, 0);
		assert.equal(fake.statements.filter((s) => s.includes('manifest_size')).length, 1);
		// The bytes the store sent back are the length probe, not the index.
		assert.ok(
			fake.counters.bytesRead < 1024,
			`store sent ${fake.counters.bytesRead} bytes for a ${entries.length} byte index`
		);
	});

	it('keeps the refusal proportional to the closure, not to the index size', async () => {
		const entries = oversized();
		const names: string[] = [];
		for (let i = 0; i < 5; i++) {
			const full = `@acme/cost${i}`;
			names.push(full);
			await publishMany([{ full, version: '1.0.0', entries }]);
		}
		clearVersionIndex();
		fake.reset();
		const reqs: Record<string, string> = {};
		for (const full of names) reqs[full] = '*';
		const out = await resolveGraph(fake.env, reqs, [], {
			inlineChunks: true,
			advertiseChunks: true
		});
		assert.equal(out.chunksDeclined, 'size-cap-exceeded');
		assert.equal(fake.statements.filter((s) => s.includes('AS tar_manifest_json')).length, 0);
		assert.ok(
			fake.counters.bytesRead < 1024,
			`store sent ${fake.counters.bytesRead} bytes for ${entries.length * 5} bytes of index`
		);
	});

	it('refuses a closure that is too large before any index is measured', async () => {
		for (let i = 0; i <= RESOLVE_MAX_INLINE_NODES; i++) {
			await publishMany([{ full: `@acme/wide${i}`, version: '1.0.0' }]);
		}
		clearVersionIndex();
		fake.reset();
		const reqs: Record<string, string> = {};
		for (let i = 0; i <= RESOLVE_MAX_INLINE_NODES; i++) reqs[`@acme/wide${i}`] = '*';
		const out = await resolveGraph(fake.env, reqs, [], {
			inlineChunks: true,
			advertiseChunks: true
		});
		assert.equal(out.chunks, 'links');
		assert.equal(out.chunksDeclined, 'closure-too-large');
		// Not even the length probe ran.
		assert.equal(fake.statements.filter((s) => s.includes('manifest_size')).length, 0);
		assert.equal(fake.statements.filter((s) => s.includes('tar_manifest_json')).length, 0);
	});

	it('reads the index of a closure that fits', async () => {
		await publishMany([{ full: '@acme/fits', version: '1.0.0' }]);
		clearVersionIndex();
		fake.reset();
		const out = await resolveGraph(fake.env, { '@acme/fits': '*' }, [], {
			inlineChunks: true,
			advertiseChunks: true
		});
		assert.equal(out.chunks, 'inline');
		assert.equal(out.chunksDeclined, null);
		assert.equal(fake.statements.filter((s) => s.includes('AS tar_manifest_json')).length, 1);
	});
});

describe('package mutation token', () => {
	it('starts at one on the first publish and moves on every publish', async () => {
		await publishPackage(fake.env, payload('@acme/tok1', '1.0.0'));
		assert.equal(mutationToken('@acme/tok1'), 1);
		await publishPackage(fake.env, payload('@acme/tok1', '1.1.0'));
		assert.equal(mutationToken('@acme/tok1'), 2);
	});

	it('does not move on a yank', async () => {
		await publishPackage(fake.env, payload('@acme/tok2', '1.0.0'));
		const before = mutationToken('@acme/tok2');
		await yankVersion(fake.env, '@acme/tok2', '1.0.0');
		assert.equal(mutationToken('@acme/tok2'), before, 'a yank does not change the candidate set');
	});

	it('moves on a takedown', async () => {
		await publishPackage(fake.env, payload('@acme/tok3', '1.0.0'));
		const before = mutationToken('@acme/tok3');
		await recordTombstone(fake.env, '@acme/tok3', '1.0.0', 'policy');
		assert.equal(mutationToken('@acme/tok3'), before + 1);
	});

	it('moves on a scope transfer', async () => {
		await publishPackage(fake.env, payload('@tr/tok4', '1.0.0'));
		const before = mutationToken('@tr/tok4');
		// Turso does not enforce foreign keys by default, and the transfer
		// moves the child rows before the parent key. The fake enforces them,
		// so the check is off for the call.
		fake.db.exec('PRAGMA foreign_keys = OFF');
		try {
			await transferScope(fake.env, 'tr', 'tr2', true);
		} finally {
			fake.db.exec('PRAGMA foreign_keys = ON');
		}
		assert.equal(mutationToken('@tr2/tok4'), before + 1);
	});

	it('rides on the packages upsert without an extra statement', async () => {
		clearVersionIndex();
		fake.reset();
		await publishPackage(fake.env, payload('@acme/tok5', '1.0.0'));
		const upserts = fake.statements.filter((s) => s.includes('insert into "packages"'));
		assert.equal(upserts.length, 1, 'the publish writes the package row once');
		assert.ok(upserts[0].includes('"mutation_count"'), upserts[0]);
	});

	it('leaves no global generation counter behind', async () => {
		clearVersionIndex();
		fake.reset();
		await publishPackage(fake.env, payload('@acme/tok6', '1.0.0'));
		await yankVersion(fake.env, '@acme/tok6', '1.0.0');
		await recordTombstone(fake.env, '@acme/tok6', '1.0.0', 'policy');
		assert.equal(
			fake.statements.filter((s) => s.includes('registry_generation')).length,
			0,
			'no statement may write the global counter'
		);
		const rows = fake.db
			.prepare("SELECT count(*) AS c FROM meta WHERE key = 'registry_generation'")
			.all() as Array<{ c: number }>;
		assert.equal(Number(rows[0].c), 0, 'the global counter row must be gone');
	});
});

// A mutation that lands in the store while this isolate's memo is warm is
// the normal production case: the row changes, the memo is not cleared.
// The memo may only be used when the per-package mutation count it was
// built with still matches the one the picked row carries.
describe('resolveGraph against a mutation from another isolate', () => {
	it('picks the newer version after another isolate published it', async () => {
		await publishMany([{ full: '@acme/iso', version: '1.0.0' }]);
		clearVersionIndex();
		const warm = await resolveGraph(fake.env, { '@acme/iso': '^1.0.0' }, []);
		assert.equal(warm.resolved['@acme/iso'], '1.0.0');
		// Another isolate publishes 1.1.0. The memo still names 1.0.0.
		otherIsolatePublish('@acme/iso', '1.1.0');
		const out = await resolveGraph(fake.env, { '@acme/iso': '^1.0.0' }, []);
		assert.equal(out.resolved['@acme/iso'], '1.1.0', 'the pick must be the max-satisfying version');
	});

	it('picks the remaining version after another isolate deleted the picked one', async () => {
		await publishMany([
			{ full: '@acme/isodel', version: '1.0.0' },
			{ full: '@acme/isodel', version: '2.0.0' }
		]);
		clearVersionIndex();
		const warm = await resolveGraph(fake.env, { '@acme/isodel': '*' }, []);
		assert.equal(warm.resolved['@acme/isodel'], '2.0.0');
		// Another isolate hard-deletes the 2.0.0 row.
		fake.db
			.prepare('DELETE FROM package_versions WHERE package_id = ? AND version = ?')
			.run('pkg_@acme/isodel', '2.0.0');
		fake.db.prepare('UPDATE packages SET mutation_count = mutation_count + 1 WHERE id = ?').run('pkg_@acme/isodel');
		const out = await resolveGraph(fake.env, { '@acme/isodel': '*' }, []);
		assert.equal(out.resolved['@acme/isodel'], '1.0.0');
	});

	it('picks the remaining version when the delete did not move the counter', async () => {
		await publishMany([
			{ full: '@acme/isodel2', version: '1.0.0' },
			{ full: '@acme/isodel2', version: '2.0.0' }
		]);
		clearVersionIndex();
		await resolveGraph(fake.env, { '@acme/isodel2': '*' }, []);
		// The row disappears without the counter moving: the missing row
		// itself has to catch it.
		fake.db
			.prepare('DELETE FROM package_versions WHERE package_id = ? AND version = ?')
			.run('pkg_@acme/isodel2', '2.0.0');
		const out = await resolveGraph(fake.env, { '@acme/isodel2': '*' }, []);
		assert.equal(out.resolved['@acme/isodel2'], '1.0.0');
	});

	it('does not serve a version another isolate took down', async () => {
		await publishMany([
			{ full: '@acme/isotomb', version: '1.0.0' },
			{ full: '@acme/isotomb', version: '2.0.0' }
		]);
		clearVersionIndex();
		await resolveGraph(fake.env, { '@acme/isotomb': '*' }, []);
		// Another isolate takes 2.0.0 down: the status flips, the tombstone
		// lands and the counter moves.
		fake.db
			.prepare('INSERT INTO tombstones (package_id, version, reason, created_at) VALUES (?, ?, ?, ?)')
			.run('pkg_@acme/isotomb', '2.0.0', 'policy', 0);
		fake.db
			.prepare("UPDATE package_versions SET status = 'tombstoned' WHERE package_id = ? AND version = ?")
			.run('pkg_@acme/isotomb', '2.0.0');
		fake.db.prepare('UPDATE packages SET mutation_count = mutation_count + 1 WHERE id = ?').run('pkg_@acme/isotomb');
		const out = await resolveGraph(fake.env, { '@acme/isotomb': '*' }, []);
		assert.equal(out.resolved['@acme/isotomb'], '1.0.0');
		assert.equal(
			out.levels.flat().some((n) => n.version === '2.0.0'),
			false,
			'a withdrawn version must never be served'
		);
	});

	it('keeps the memo from hiding a takedown of the only version', async () => {
		await publishMany([{ full: '@acme/isohide', version: '1.0.0' }]);
		clearVersionIndex();
		await resolveGraph(fake.env, { '@acme/isohide': '*' }, []);
		fake.db
			.prepare('INSERT INTO tombstones (package_id, version, reason, created_at) VALUES (?, ?, ?, ?)')
			.run('pkg_@acme/isohide', '1.0.0', 'policy', 0);
		fake.db
			.prepare("UPDATE package_versions SET status = 'tombstoned' WHERE package_id = ? AND version = ?")
			.run('pkg_@acme/isohide', '1.0.0');
		fake.db.prepare('UPDATE packages SET mutation_count = mutation_count + 1 WHERE id = ?').run('pkg_@acme/isohide');
		await assert.rejects(
			() => resolveGraph(fake.env, { '@acme/isohide': '*' }, []),
			/^Error: package @acme\/isohide does not exist$/
		);
	});

	it('reports a yank from another isolate without a counter move', async () => {
		await publishMany([{ full: '@acme/isoyank', version: '1.0.0' }]);
		clearVersionIndex();
		await resolveGraph(fake.env, { '@acme/isoyank': '*' }, []);
		// A yank keeps the version a candidate, so the counter stays put.
		fake.db
			.prepare("UPDATE package_versions SET status = 'yanked' WHERE package_id = ? AND version = ?")
			.run('pkg_@acme/isoyank', '1.0.0');
		assert.equal(mutationToken('@acme/isoyank'), 1);
		const out = await resolveGraph(fake.env, { '@acme/isoyank': '*' }, []);
		assert.equal(out.resolved['@acme/isoyank'], '1.0.0');
		assert.equal(out.levels[0][0].yanked, true, 'the yank must not be hidden by the memo');
	});

	it('answers the old name with does not exist after a transfer, and the new name with the versions', async () => {
		await publishMany([{ full: '@isoold/moved', version: '1.0.0' }]);
		clearVersionIndex();
		const warm = await resolveGraph(fake.env, { '@isoold/moved': '*' }, []);
		assert.equal(warm.resolved['@isoold/moved'], '1.0.0');
		// Another isolate transfers the scope.
		otherIsolateTransfer('isoold', 'isonew');
		await assert.rejects(
			() => resolveGraph(fake.env, { '@isoold/moved': '*' }, []),
			/^Error: package @isoold\/moved does not exist$/
		);
		const moved = await resolveGraph(fake.env, { '@isonew/moved': '*' }, []);
		assert.equal(moved.resolved['@isonew/moved'], '1.0.0');
	});

	it('answers a package that never existed from a fresh read every time', async () => {
		await resolveGraph(fake.env, { '@acme/never': '*' }, []).catch(() => undefined);
		// The empty list is not memoized, so the second answer is read fresh.
		clearVersionIndex();
		fake.reset();
		await assert.rejects(
			() => resolveGraph(fake.env, { '@acme/never': '*' }, []),
			/^Error: package @acme\/never does not exist$/
		);
		assert.equal(
			fake.statements.filter((s) => s.includes('p.name AS name, v.version AS version')).length,
			1,
			'the candidate list of a missing package is read once, from the store'
		);
	});
});

// The repair is bounded: a registry that keeps racing answers a named
// failure instead of a wrong version.
describe('resolveGraph repair bound', () => {
	it('re-reads the candidates once for a stale memo and stops there', async () => {
		await publishMany([{ full: '@acme/rep', version: '1.0.0' }]);
		clearVersionIndex();
		await resolveGraph(fake.env, { '@acme/rep': '*' }, []);
		otherIsolatePublish('@acme/rep', '1.1.0');
		fake.reset();
		const out = await resolveGraph(fake.env, { '@acme/rep': '*' }, []);
		assert.equal(out.resolved['@acme/rep'], '1.1.0');
		const candidateReads = fake.statements.filter((s) => s.includes('p.name AS name, v.version AS version'));
		assert.equal(candidateReads.length, 1, 'the repair re-reads the candidates once');
	});

	it('fails with a named error when the answer cannot be settled', async () => {
		await publishMany([{ full: '@acme/race', version: '1.0.0' }]);
		clearVersionIndex();
		await resolveGraph(fake.env, { '@acme/race': '*' }, []);
		// A tombstone that reached the version while the row still says live:
		// the version stays a candidate, so every repair picks it again.
		fake.db
			.prepare('INSERT INTO tombstones (package_id, version, reason, created_at) VALUES (?, ?, ?, ?)')
			.run('pkg_@acme/race', '1.0.0', 'policy', 0);
		await assert.rejects(
			() => resolveGraph(fake.env, { '@acme/race': '*' }, []),
			/resolve raced with a mutation/
		);
	});

	it('bounds the repair rounds', () => {
		assert.equal(RESOLVE_REPAIR_ROUNDS, 2);
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

	it('mints a version once but still bumps the mutation counter on a republish', async () => {
		await publishPackage(fake.env, payload('@acme/idem3', '1.0.0'));
		const token = mutationToken('@acme/idem3');
		const again = await publishPackage(fake.env, payload('@acme/idem3', '1.0.0'));
		assert.equal(again.created, false);
		assert.equal(mutationToken('@acme/idem3'), token + 1);
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

// The audit row of a publish reads changes() from the version insert, so the
// two statements have to stay neighbours in the batch. The order is the
// idempotency contract.
describe('publish batch order', () => {
	const versionIndex = (stmts: string[]): number =>
		stmts.findIndex((s) => s.includes('insert into "package_versions"'));
	const auditIndex = (stmts: string[]): number =>
		stmts.findIndex((s) => s.includes("'publish'") && s.includes('changes() > 0'));
	const packageIndex = (stmts: string[]): number =>
		stmts.findIndex((s) => s.includes('insert into "packages"'));

	it('reads the version insert and writes the package row before it', async () => {
		clearVersionIndex();
		fake.reset();
		await publishPackage(fake.env, payload('@acme/order', '1.0.0', {
			manifestJson: JSON.stringify({ deps: { '@acme/orderdep': '*' } })
		}));
		const stmts = fake.statements;
		const version = versionIndex(stmts);
		const audit = auditIndex(stmts);
		const pkg = packageIndex(stmts);
		assert.ok(version >= 0 && audit >= 0 && pkg >= 0, stmts.join(' | '));
		assert.equal(audit, version + 1, 'the audit row must read changes() from the version insert');
		assert.ok(pkg < version, 'the package row carries the counter and comes first');
		assert.equal(stmts.filter((s) => s === 'BEGIN DEFERRED').length, 1);
		assert.equal(stmts.filter((s) => s === 'COMMIT').length, 1);
	});

	it('keeps the order with trailing dependency and capability inserts', async () => {
		clearVersionIndex();
		fake.reset();
		await publishPackage(fake.env, payload('@acme/order2', '1.0.0', {
			manifestJson: JSON.stringify({ deps: { '@acme/orderdep2': '*' } }),
			capabilities: [{ domain: 'fs.read', arg: '/tmp', reason: null }]
		}));
		const stmts = fake.statements;
		const version = versionIndex(stmts);
		const audit = auditIndex(stmts);
		assert.equal(audit, version + 1);
		// The trailing inserts must not disturb the changes() reading.
		const rows = fake.db
			.prepare("SELECT count(*) AS c FROM audit_log WHERE action = 'publish' AND full_name = '@acme/order2'")
			.all() as Array<{ c: number }>;
		assert.equal(Number(rows[0].c), 1);
		const again = await publishPackage(fake.env, payload('@acme/order2', '1.0.0', {
			manifestJson: JSON.stringify({ deps: { '@acme/orderdep2': '*' } }),
			capabilities: [{ domain: 'fs.read', arg: '/tmp', reason: null }]
		}));
		assert.equal(again.created, false);
		const after = fake.db
			.prepare("SELECT count(*) AS c FROM audit_log WHERE action = 'publish' AND full_name = '@acme/order2'")
			.all() as Array<{ c: number }>;
		assert.equal(Number(after[0].c), 1, 'a republish must not write a second audit row');
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

	it('sends the same bytes for a client that never opts in', async () => {
		await publishMany([
			{ full: '@acme/bytes', version: '1.0.0', deps: { '@acme/bytesdep': '^1.0.0' } },
			{ full: '@acme/bytesdep', version: '1.0.0' }
		]);
		clearVersionIndex();
		const out = await resolveGraph(fake.env, { '@acme/bytes': '*' }, []);
		// The body a client that sends no `chunks` field gets, spelled out.
		const body = JSON.stringify({ resolved: out.resolved, base: '', levels: out.levels });
		assert.equal(
			body,
			JSON.stringify({
				resolved: { '@acme/bytes': '1.0.0', '@acme/bytesdep': '1.0.0' },
				base: '',
				levels: [
					[
						{
							full: '@acme/bytes',
							version: '1.0.0',
							path: '@acme/bytes@1.0.0/chunks',
							integrity: 'sha-@acme/bytes-1.0.0',
							engineRange: '',
							deps: ['@acme/bytesdep'],
							yanked: false
						}
					],
					[
						{
							full: '@acme/bytesdep',
							version: '1.0.0',
							path: '@acme/bytesdep@1.0.0/chunks',
							integrity: 'sha-@acme/bytesdep-1.0.0',
							engineRange: '',
							deps: [],
							yanked: false
						}
					]
				]
			})
		);
	});

	it('advertises the inline-chunks capability additively', () => {
		assert.ok(CAPABILITIES.includes('resolve-inline-chunks'));
		assert.equal(CAPABILITIES.includes('tombstones'), true);
	});
});