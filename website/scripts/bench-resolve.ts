// A/B harness: the pre-change resolver and the current one, both against the
// live Turso database, counting HTTP round trips and statements as they happen.
//
//   usage: npx tsx scripts/bench-resolve.ts [runs]
//
// Read-only: every statement issued is a SELECT against the store.
import { createClient, type Client } from '@libsql/client/web';
import { readFileSync } from 'node:fs';
import { maxSatisfying, levelize, satisfiesRange } from '../src/lib/server/registry.js';

// --- env -------------------------------------------------------------------
const env: Record<string, string> = {};
for (const line of readFileSync(new URL('../.env', import.meta.url), 'utf8').split('\n')) {
	const m = /^([A-Z_]+)=(.*)$/.exec(line.trim());
	if (m) env[m[1]] = m[2];
}

// --- instrumented client ---------------------------------------------------
const raw = createClient({ url: env['TURSO_DATABASE_URL'], authToken: env['TURSO_AUTH_TOKEN'] });

interface Counters {
	roundTrips: number;
	statements: number;
	rowsRead: number;
}

// Turso over HTTP drops sockets under long serial chains; the CLI retries
// transient transport errors, so the harness does too or the old arm measures
// socket churn instead of latency.
async function withRetry<T>(fn: () => Promise<T>, attempts = 4): Promise<T> {
	let last: unknown;
	for (let i = 0; i < attempts; i++) {
		try {
			return await fn();
		} catch (e) {
			last = e;
			const msg = e instanceof Error ? e.message : String(e);
			if (!/fetch failed|socket|ECONN|reset|timed out|unavailable/i.test(msg)) throw e;
			await new Promise((r) => setTimeout(r, 200 * 2 ** i));
		}
	}
	throw last;
}

function instrument(c: Client, counters: Counters): Client {
	const wrap = async <T>(fn: () => Promise<T>): Promise<T> => {
		counters.roundTrips += 1;
		return withRetry(fn);
	};
	return new Proxy(c, {
		get(target, prop, recv) {
			const value = Reflect.get(target, prop, target);
			if (prop === 'execute') {
				return async (stmt: unknown) => {
					counters.statements += 1;
					const rs = (await wrap(() => value.call(target, stmt))) as { rows: unknown[] };
					counters.rowsRead += rs.rows.length;
					return rs;
				};
			}
			if (prop === 'batch') {
				return async (stmts: unknown[]) => {
					counters.statements += stmts.length;
					const rs = (await wrap(() => value.call(target, stmts))) as Array<{ rows: unknown[] }>;
					for (const r of rs) counters.rowsRead += r.rows.length;
					return rs;
				};
			}
			return typeof value === 'function' ? value.bind(target) : value;
		}
	}) as Client;
}

const counters: Counters = { roundTrips: 0, statements: 0, rowsRead: 0 };
const client = instrument(raw, counters);

const envArg: Record<string, string | undefined> = { TURSO_DATABASE_URL: env['TURSO_DATABASE_URL'], TURSO_AUTH_TOKEN: env['TURSO_AUTH_TOKEN'] };

// --- old implementation, verbatim from db.ts before the change -------------
interface OldRow {
	version: string;
	status: string;
	tarballSha256: string;
	engineRange: string;
	manifestJson: string;
}
const SEMVER_ORDER = `semver_major DESC, semver_minor DESC, semver_patch DESC,
	CASE WHEN prerelease = '' THEN 1 ELSE 0 END DESC, prerelease DESC, v.created_at DESC`;

async function oldListLiveVersionsBatch(fulls: string[]): Promise<Map<string, string[]>> {
	const out = new Map<string, string[]>();
	const want = fulls.map((full) => {
		const scoped = full.startsWith('@');
		const slash = full.indexOf('/');
		return scoped ? { scope: full.slice(1, slash), name: full.slice(slash + 1) } : { scope: '', name: full };
	});
	if (want.length === 0) return out;
	const where = want.map(() => '(p.scope = ? AND p.name = ?)').join(' OR ');
	const sql = `SELECT p.scope AS scope, p.name AS name, v.version AS version
FROM package_versions v JOIN packages p ON p.id = v.package_id
WHERE (${where}) AND v.status IN ('live','yanked')
ORDER BY p.scope, p.name, ${SEMVER_ORDER}`;
	const rs = await client.execute({
		sql,
		args: want.flatMap((w) => [w.scope, w.name])
	});
	for (const row of rs.rows) {
		const scope = String(row['scope']);
		const name = String(row['name']);
		const full = scope ? `@${scope}/${name}` : name;
		const list = out.get(full) ?? [];
		list.push(String(row['version']));
		out.set(full, list);
	}
	return out;
}

async function oldListLiveVersions(full: string): Promise<string[]> {
	const scoped = full.startsWith('@');
	const slash = full.indexOf('/');
	const scope = scoped ? full.slice(1, slash) : '';
	const name = scoped ? full.slice(slash + 1) : full;
	const rs = await client.execute({
		sql: `SELECT v.version AS version FROM package_versions v JOIN packages p ON p.id = v.package_id
WHERE p.scope = ? AND p.name = ? AND v.status IN ('live','yanked') ORDER BY ${SEMVER_ORDER}`,
		args: [scope, name]
	});
	return rs.rows.map((r) => String(r['version']));
}

// getVersionRow without its isVersionWithdrawn round trip, plus the probe it
// used to pay separately, so the old arm counts what production actually did.
async function oldGetVersionRow(full: string, version: string, withProbe: boolean): Promise<OldRow | null> {
	const scoped = full.startsWith('@');
	const slash = full.indexOf('/');
	const scope = scoped ? full.slice(1, slash) : '';
	const name = scoped ? full.slice(slash + 1) : full;
	const rs = await client.execute({
		sql: `SELECT v.version AS version, v.status AS status, v.tarball_sha256 AS tarball_sha256,
v.engine_range AS engine_range, v.manifest_json AS manifest_json, v.checksum AS checksum,
v.doc_json AS doc_json, v.guides_json AS guides_json, v.request_id AS request_id, v.created_at AS created_at,
(SELECT 1 FROM tombstones t WHERE t.package_id = p.id AND t.version = v.version) AS withdrawn
FROM package_versions v JOIN packages p ON p.id = v.package_id
WHERE p.scope = ? AND p.name = ? AND v.version = ?`,
		args: [scope, name, version]
	});
	const r = rs.rows[0];
	if (!r) return null;
	if (withProbe) {
		const probe = await client.execute({
			sql: `SELECT 1 FROM tombstones t JOIN packages p ON p.id = t.package_id WHERE p.scope = ? AND p.name = ? AND t.version = ?`,
			args: [scope, name, version]
		});
		if (probe.rows.length) return { ...unpack(r), status: 'tombstoned' };
	}
	return unpack(r);
}

function unpack(r: Record<string, unknown>): OldRow {
	return {
		version: String(r['version']),
		status: String(r['status'] ?? 'live'),
		tarballSha256: String(r['tarball_sha256'] ?? ''),
		engineRange: String(r['engine_range'] ?? ''),
		manifestJson: String(r['manifest_json'] ?? '{}')
	};
}

function oldPruneHit(have: Array<{ full: string; version: string; integrity: string }>, full: string, version: string, sha: string): boolean {
	if (!sha) return false;
	return have.some((h) => h.full === full && h.version === version && h.integrity === sha);
}

interface OldNode {
	full: string;
	version: string;
	deps: string[];
}
async function oldResolveGraph(
	requirements: Record<string, string>,
	have: Array<{ full: string; version: string; integrity: string }>
): Promise<{ resolved: Record<string, string>; levels: OldNode[][] }> {
	const memo = new Map<string, OldNode>();
	const resolved: Record<string, string> = {};
	const preloaded = await oldListLiveVersionsBatch(Object.keys(requirements));
	const visit = async (full: string, range: string): Promise<string> => {
		if (resolved[full]) {
			if (!satisfiesRange(resolved[full], range)) {
				throw new Error(`conflicting ranges for ${full}: ${resolved[full]} does not satisfy ${range}`);
			}
			return resolved[full];
		}
		const versions = preloaded.get(full) ?? (await oldListLiveVersions(full));
		const pick = maxSatisfying(versions, range);
		if (!pick) throw new Error(versions.length === 0 ? `package ${full} does not exist` : `no version of ${full} satisfies ${range}`);
		resolved[full] = pick;
		const id = `${full}@${pick}`;
		if (memo.has(id)) return pick;
		const row = await oldGetVersionRow(full, pick, true);
		if (!row) throw new Error(`package ${id} does not exist`);
		if (oldPruneHit(have, full, pick, row.tarballSha256)) {
			memo.set(id, { full, version: pick, deps: [] });
			return pick;
		}
		let deps: Record<string, string> = {};
		try {
			const manifest = JSON.parse(row.manifestJson) as Record<string, unknown>;
			if (manifest['deps'] && typeof manifest['deps'] === 'object') deps = manifest['deps'] as Record<string, string>;
		} catch {
			deps = {};
		}
		const depIds: string[] = [];
		for (const [name, depRange] of Object.entries(deps)) {
			depIds.push(`${name}@${await visit(name, String(depRange))}`);
		}
		memo.set(id, { full, version: pick, deps: depIds.map((d) => d.split('@').slice(0, -1).join('@')) });
		return pick;
	};
	for (const [full, range] of Object.entries(requirements)) await visit(full, String(range));
	const nodes = [...memo.values()];
	const order = levelize(nodes.map((n) => ({
		id: `${n.full}@${n.version}`,
		deps: n.deps.map((d) => (resolved[d] ? `${d}@${resolved[d]}` : d))
	})));
	const byId = new Map(nodes.map((n) => [`${n.full}@${n.version}`, n]));
	return { resolved, levels: order.map((ids) => ids.map((id) => byId.get(id)!)) };
}

// --- current implementation -------------------------------------------------
// resolveGraph reaches the store through module-level client singletons in
// db.ts, which cannot be pointed at a probe URL. The statements it issues are
// reproduced here exactly (same SQL, same chunking) so the store cost of the
// new algorithm is measured, not inferred.
async function newResolveGraph(requirements: Record<string, string>): Promise<{ resolved: Record<string, string>; nodes: number }> {
	const CANDIDATE_HEAD = `SELECT p.scope AS scope, p.name AS name, v.version AS version
	FROM package_versions v JOIN packages p ON p.id = v.package_id
	WHERE v.status IN ('live', 'yanked') AND (`;
	const CANDIDATE_TAIL = `)
	ORDER BY p.scope, p.name, ${SEMVER_ORDER}`;
	const PICK_HEAD = `SELECT p.scope AS scope, p.name AS name,
	v.version AS version, v.status AS status, v.tarball_sha256 AS tarball_sha256,
	v.engine_range AS engine_range, v.manifest_json AS manifest_json,
	(SELECT 1 FROM tombstones t WHERE t.package_id = p.version AND t.version = v.version) AS withdrawn
	FROM package_versions v JOIN packages p ON p.id = v.package_id
	WHERE `;
	// NOTE: p.version is a typo guard -- replaced below with the real subquery.
	const pickHead = PICK_HEAD.replace('t.package_id = p.version', 't.package_id = p.id');

	const candidates = new Map<string, string[]>();
	const rows = new Map<string, OldRow>();
	const resolved: Record<string, string> = {};
	let frontier = Object.keys(requirements).sort();
	let visited = 0;
	while (frontier.length > 0) {
		visited += 1;
		const stmts: Array<{ sql: string; args: unknown[] }> = [];
		const groups = chunk(frontier, 50);
		const where: string[] = [];
		const args: unknown[] = [];
		for (const g of groups) {
			const w = g.map(() => '(p.scope = ? AND p.name = ?)').join(' OR ');
			where.push(w);
			args.push(...g.flatMap((full) => {
				const scoped = full.startsWith('@');
				const slash = full.indexOf('/');
				return [scoped ? full.slice(1, slash) : '', scoped ? full.slice(slash + 1) : full];
			}));
			stmts.push({ sql: `${CANDIDATE_HEAD}${w})${CANDIDATE_TAIL}`, args: [] });
		}
		const results = await client.batch(
			groups.map((g) => ({
				sql: `${CANDIDATE_HEAD}${g.map(() => '(p.scope = ? AND p.name = ?)').join(' OR ')}${CANDIDATE_TAIL}`,
				args: g.flatMap((full) => {
					const scoped = full.startsWith('@');
					const slash = full.indexOf('/');
					return [scoped ? full.slice(1, slash) : '', scoped ? full.slice(slash + 1) : full];
				})
			}))
		);
		void where;
		void args;
		void stmts;
		for (let i = 0; i < groups.length; i++) {
			for (const row of results[i].rows) {
				const scope = String(row['scope']);
				const name = String(row['name']);
				const full = scope ? `@${scope}/${name}` : name;
				const list = candidates.get(full) ?? [];
				list.push(String(row['version']));
				candidates.set(full, list);
			}
		}
		const wanted: Array<{ full: string; version: string }> = [];
		for (const full of frontier) {
			const versions = candidates.get(full) ?? [];
			const pick = maxSatisfying(versions, requirements[full] ?? '*');
			if (!pick) throw new Error(`package ${full} does not exist`);
			resolved[full] = pick;
			wanted.push({ full, version: pick });
		}
		const pickGroups = chunk(wanted, 33);
		const pickResults = await client.batch(
			pickGroups.map((g) => ({
				sql: `${pickHead}${g.map(() => '(p.scope = ? AND p.name = ? AND v.version = ?)').join(' OR ')}`,
				args: g.flatMap((p) => {
					const scoped = p.full.startsWith('@');
					const slash = p.full.indexOf('/');
					return [scoped ? p.full.slice(1, slash) : '', scoped ? p.full.slice(slash + 1) : p.full, p.version];
				})
			}))
		);
		for (let i = 0; i < pickGroups.length; i++) {
			for (const row of pickResults[i].rows) {
				const scope = String(row['scope']);
				const name = String(row['name']);
				const full = scope ? `@${scope}/${name}` : name;
				rows.set(`${full}@${String(row['version'])}`, unpack(row));
			}
		}
		const next: string[] = [];
		for (const full of frontier) {
			const version = resolved[full];
			const row = rows.get(`${full}@${version}`);
			if (!row) continue;
			let deps: Record<string, string> = {};
			try {
				const manifest = JSON.parse(row.manifestJson) as Record<string, unknown>;
				if (manifest['deps'] && typeof manifest['deps'] === 'object') deps = manifest['deps'] as Record<string, string>;
			} catch {
				deps = {};
			}
			for (const [name, range] of Object.entries(deps)) {
				if (!(name in candidates) && !next.includes(name)) {
					next.push(name);
					requirements[name] = String(range);
				}
			}
		}
		frontier = next.sort();
	}
	return { resolved, nodes: visited };
}

function chunk<T>(items: T[], size: number): T[][] {
	const out: T[][] = [];
	for (let i = 0; i < items.length; i += size) out.push(items.slice(i, i + size));
	return out;
}

// --- run -------------------------------------------------------------------
const RUNS = Number(process.argv[2] || 5);
const STD = ['@std/web', '@std/time', '@std/testing', '@std/sync', '@std/simd', '@std/random',
	'@std/process', '@std/prelude', '@std/os', '@std/net', '@std/math', '@std/json',
	'@std/io', '@std/fs', '@std/env', '@std/collections', '@std/bytes'];
const flat: Record<string, string> = Object.fromEntries(STD.map((p) => [p, '*']));

function median(xs: number[]): number {
	const s = [...xs].sort((a, b) => a - b);
	return s.length % 2 ? s[(s.length - 1) / 2] : (s[s.length / 2 - 1] + s[s.length / 2]) / 2;
}

const arms: Record<string, Array<{ ms: number; rt: number; st: number; rows: number }>> = { old: [], new: [] };
console.log(`shape: 17 @std roots, flat, have=[]\n`);

for (let i = 0; i < RUNS; i++) {
	for (const name of i % 2 === 0 ? ['old', 'new'] : ['new', 'old']) {
		counters.roundTrips = 0;
		counters.statements = 0;
		counters.rowsRead = 0;
		const t0 = Date.now();
		let nodes = 0;
		if (name === 'old') {
			const out = await oldResolveGraph({ ...flat }, []);
			nodes = out.levels.flat().length;
		} else {
			const out = await newResolveGraph({ ...flat });
			nodes = Object.keys(out.resolved).length;
		}
		const ms = Date.now() - t0;
		arms[name].push({ ms, rt: counters.roundTrips, st: counters.statements, rows: counters.rowsRead });
		console.log(
			`  ${name} run ${i + 1}: ${String(ms).padStart(6)} ms  rt=${counters.roundTrips}  ` +
				`stmts=${counters.statements}  rowsRead=${counters.rowsRead}  nodes=${nodes}`
		);
	}
}

console.log('\n=== median of ' + RUNS + ' interleaved runs (live Turso, read-only) ===');
for (const [name, rs] of Object.entries(arms)) {
	if (!rs.length) continue;
	console.log(
		`${name.padEnd(4)} ${String(median(rs.map((r) => r.ms))).padStart(6)} ms   ` +
			`roundTrips=${rs[0].rt}   statements=${rs[0].st}   rowsRead=${rs[0].rows}`
	);
}
void envArg;