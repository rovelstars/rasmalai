// Synthetic deeper graph: N nodes over L levels, same A/B shape as
// bench-resolve.ts but against a local in-memory store, so the statement and
// round-trip counts are exact and not dominated by transatlantic RTT.
//
//   usage: npx tsx scripts/bench-deep.ts
import { DatabaseSync } from 'node:sqlite';
import { maxSatisfying, levelize, satisfiesRange } from '../src/lib/server/registry.js';

const SEMVER_ORDER = `semver_major DESC, semver_minor DESC, semver_patch DESC,
	CASE WHEN prerelease = '' THEN 1 ELSE 0 END DESC, prerelease DESC, v.created_at DESC`;

interface Counter {
	statements: number;
	roundTrips: number;
	rowsRead: number;
}

class Store {
	counter: Counter = { statements: 0, roundTrips: 0, rowsRead: 0 };
	constructor(readonly db: DatabaseSync) {}
	async execute(sql: string, args: unknown[]): Promise<Record<string, unknown>[]> {
		this.counter.statements += 1;
		this.counter.roundTrips += 1;
		const rows = this.db.prepare(sql).all(...(args as never[])) as Record<string, unknown>[];
		this.counter.rowsRead += rows.length;
		return rows;
	}
	async batch(stmts: Array<{ sql: string; args: unknown[] }>): Promise<Array<Record<string, unknown>[]>> {
		this.counter.statements += stmts.length;
		this.counter.roundTrips += 1;
		const out: Array<Record<string, unknown>[]> = [];
		this.db.exec('BEGIN DEFERRED');
		try {
			for (const s of stmts) {
				const rows = this.db.prepare(s.sql).all(...(s.args as never[])) as Record<string, unknown>[];
				this.counter.rowsRead += rows.length;
				out.push(rows);
			}
		} finally {
			this.db.exec('COMMIT');
		}
		return out;
	}
}

function split(full: string): { scope: string; name: string } {
	if (full.startsWith('@')) {
		const slash = full.indexOf('/');
		return { scope: full.slice(1, slash), name: full.slice(slash + 1) };
	}
	return { scope: '', name: full };
}

// --- old -------------------------------------------------------------------
async function oldListBatch(store: Store, fulls: string[]): Promise<Map<string, string[]>> {
	const out = new Map<string, string[]>();
	const want = fulls.map(split);
	if (!want.length) return out;
	const rows = await store.execute(
		`SELECT p.scope AS scope, p.name AS name, v.version AS version
FROM package_versions v JOIN packages p ON p.id = v.package_id
WHERE (${want.map(() => '(p.scope = ? AND p.name = ?)').join(' OR ')}) AND v.status IN ('live','yanked')
ORDER BY p.scope, p.name, ${SEMVER_ORDER}`,
		want.flatMap((w) => [w.scope, w.name])
	);
	for (const row of rows) {
		const scope = String(row['scope']);
		const name = String(row['name']);
		const full = scope ? `@${scope}/${name}` : name;
		const list = out.get(full) ?? [];
		list.push(String(row['version']));
		out.set(full, list);
	}
	return out;
}

async function oldListOne(store: Store, full: string): Promise<string[]> {
	const { scope, name } = split(full);
	const rows = await store.execute(
		`SELECT v.version AS version FROM package_versions v JOIN packages p ON p.id = v.package_id
WHERE p.scope = ? AND p.name = ? AND v.status IN ('live','yanked') ORDER BY ${SEMVER_ORDER}`,
		[scope, name]
	);
	return rows.map((r) => String(r['version']));
}

async function oldRow(store: Store, full: string, version: string): Promise<{ manifestJson: string } | null> {
	const { scope, name } = split(full);
	const rows = await store.execute(
		`SELECT v.manifest_json AS manifest_json FROM package_versions v JOIN packages p ON p.id = v.package_id
WHERE p.scope = ? AND p.name = ? AND v.version = ?`,
		[scope, name, version]
	);
	if (!rows.length) return null;
	// the discarded isVersionWithdrawn probe
	await store.execute(
		`SELECT 1 FROM tombstones t JOIN packages p ON p.id = t.package_id WHERE p.scope = ? AND p.name = ? AND t.version = ?`,
		[scope, name, version]
	);
	return { manifestJson: String(rows[0]['manifest_json'] ?? '{}') };
}

async function oldResolve(store: Store, requirements: Record<string, string>) {
	const memo = new Map<string, { full: string; version: string; deps: string[] }>();
	const resolved: Record<string, string> = {};
	const preloaded = await oldListBatch(store, Object.keys(requirements));
	const visit = async (full: string, range: string): Promise<string> => {
		if (resolved[full]) {
			if (!satisfiesRange(resolved[full], range)) {
				throw new Error(`conflicting ranges for ${full}: ${resolved[full]} does not satisfy ${range}`);
			}
			return resolved[full];
		}
		const versions = preloaded.get(full) ?? (await oldListOne(store, full));
		const pick = maxSatisfying(versions, range);
		if (!pick) throw new Error(`package ${full} does not exist`);
		resolved[full] = pick;
		const id = `${full}@${pick}`;
		if (memo.has(id)) return pick;
		const row = await oldRow(store, full, pick);
		if (!row) throw new Error(`package ${id} does not exist`);
		let deps: Record<string, string> = {};
		try {
			const m = JSON.parse(row.manifestJson) as Record<string, unknown>;
			if (m['deps'] && typeof m['deps'] === 'object') deps = m['deps'] as Record<string, string>;
		} catch {
			deps = {};
		}
		const depIds: string[] = [];
		for (const [name, depRange] of Object.entries(deps)) depIds.push(`${name}@${await visit(name, String(depRange))}`);
		memo.set(id, { full, version: pick, deps: depIds.map((d) => d.split('@').slice(0, -1).join('@')) });
		return pick;
	};
	for (const [full, range] of Object.entries(requirements)) await visit(full, String(range));
	const nodes = [...memo.values()];
	const order = levelize(nodes.map((n) => ({
		id: `${n.full}@${n.version}`,
		deps: n.deps.map((d) => (resolved[d] ? `${d}@${resolved[d]}` : d))
	})));
	return { resolved, nodes: nodes.length, levels: order.length };
}

// --- new -------------------------------------------------------------------
async function newResolve(store: Store, requirements: Record<string, string>) {
	const CANDIDATE_HEAD = `SELECT p.scope AS scope, p.name AS name, v.version AS version
	FROM package_versions v JOIN packages p ON p.id = v.package_id
	WHERE v.status IN ('live', 'yanked') AND (`;
	const CANDIDATE_TAIL = `)
	ORDER BY p.scope, p.name, ${SEMVER_ORDER}`;
	const PICK_HEAD = `SELECT p.scope AS scope, p.name AS name,
	v.version AS version, v.status AS status, v.tarball_sha256 AS tarball_sha256,
	v.engine_range AS engine_range, v.manifest_json AS manifest_json,
	(SELECT 1 FROM tombstones t WHERE t.package_id = p.id AND t.version = v.version) AS withdrawn
	FROM package_versions v JOIN packages p ON p.id = v.package_id
	WHERE `;

	const candidates = new Map<string, string[]>();
	const rows = new Map<string, { manifestJson: string }>();
	const resolved: Record<string, string> = {};
	const wanted = new Map<string, string>();
	for (const [f, r] of Object.entries(requirements)) wanted.set(f, r);
	let frontier = [...wanted.keys()].sort();
	while (frontier.length) {
		const g = chunk(frontier, 50);
		const results = await store.batch(
			g.map((grp) => ({
				sql: `${CANDIDATE_HEAD}${grp.map(() => '(p.scope = ? AND p.name = ?)').join(' OR ')}${CANDIDATE_TAIL}`,
				args: grp.flatMap((f) => { const s = split(f); return [s.scope, s.name]; })
			}))
		);
		for (const set of results) {
			for (const row of set) {
				const scope = String(row['scope']);
				const name = String(row['name']);
				const full = scope ? `@${scope}/${name}` : name;
				const list = candidates.get(full) ?? [];
				list.push(String(row['version']));
				candidates.set(full, list);
			}
		}
		const picks: Array<{ full: string; version: string }> = [];
		for (const full of frontier) {
			const versions = candidates.get(full) ?? [];
			const pick = maxSatisfying(versions, wanted.get(full)!);
			if (!pick) throw new Error(`package ${full} does not exist`);
			resolved[full] = pick;
			picks.push({ full, version: pick });
		}
		const pg = chunk(picks, 33);
		const picksOut = await store.batch(
			pg.map((grp) => ({
				sql: `${PICK_HEAD}${grp.map(() => '(p.scope = ? AND p.name = ? AND v.version = ?)').join(' OR ')}`,
				args: grp.flatMap((p) => { const s = split(p.full); return [s.scope, s.name, p.version]; })
			}))
		);
		for (const set of picksOut) {
			for (const row of set) {
				const scope = String(row['scope']);
				const name = String(row['name']);
				const full = scope ? `@${scope}/${name}` : name;
				rows.set(`${full}@${String(row['version'])}`, { manifestJson: String(row['manifest_json'] ?? '{}') });
			}
		}
		const next: string[] = [];
		for (const full of frontier) {
			const row = rows.get(`${full}@${resolved[full]}`);
			if (!row) continue;
			let deps: Record<string, string> = {};
			try {
				const m = JSON.parse(row.manifestJson) as Record<string, unknown>;
				if (m['deps'] && typeof m['deps'] === 'object') deps = m['deps'] as Record<string, string>;
			} catch {
				deps = {};
			}
			for (const [name, range] of Object.entries(deps)) {
				if (!wanted.has(name)) {
					wanted.set(name, String(range));
					next.push(name);
				}
			}
		}
		frontier = next.sort();
	}
	return { resolved, nodes: Object.keys(resolved).length };
}

function chunk<T>(items: T[], size: number): T[][] {
	const out: T[][] = [];
	for (let i = 0; i < items.length; i += size) out.push(items.slice(i, i + size));
	return out;
}

// --- fixture ---------------------------------------------------------------
function seed(roots: number, levels: number, fanout: number): { db: DatabaseSync; requirements: Record<string, string> } {
	const db = new DatabaseSync(':memory:');
	db.exec(`CREATE TABLE packages (id TEXT PRIMARY KEY, scope TEXT, name TEXT);
CREATE TABLE package_versions (id TEXT PRIMARY KEY, package_id TEXT, version TEXT, status TEXT,
  semver_major INT, semver_minor INT, semver_patch INT, prerelease TEXT, created_at INT,
  engine_range TEXT, manifest_json TEXT, tarball_sha256 TEXT, tar_manifest_json TEXT);
CREATE TABLE tombstones (package_id TEXT, version TEXT, PRIMARY KEY (package_id, version));`);
	const ins = db.prepare('INSERT INTO packages (id, scope, name) VALUES (?, ?, ?)');
	const insv = db.prepare(`INSERT INTO package_versions (id, package_id, version, status,
  semver_major, semver_minor, semver_patch, prerelease, created_at, engine_range, manifest_json, tarball_sha256, tar_manifest_json)
  VALUES (?, ?, ?, 'live', 1, 0, 0, '', 1, '', ?, 'sha', '[]')`);
	const next = (i: number) => `@acme/p${i}`;
	let counter = 0;
	// level 0 roots: roots distinct packages
	const rootNames: string[] = [];
	for (let i = 0; i < roots; i++) rootNames.push(next(counter++));
	// levels 1..levels-1: fanout new packages each, wired level by level
	let frontier = rootNames;
	for (let l = 1; l < levels; l++) {
		const nextLevel: string[] = [];
		for (const parent of frontier) {
			const kids: string[] = [];
			for (let f = 0; f < fanout; f++) kids.push(next(counter++));
			nextLevel.push(...kids);
			pendingDeps.set(parent, kids);
		}
		frontier = nextLevel;
	}
	const pendingDeps = new Map<string, string[]>();
	void pendingDeps;
	return { db, requirements: Object.fromEntries(rootNames.map((n) => [n, '*'])) };
}

// The seed above needs the dependency map first; build it explicitly instead.
function buildGraph(roots: number, levels: number, fanout: number) {
	const names: string[] = [];
	let n = 0;
	const mk = () => `@acme/p${n++}`;
	const deps = new Map<string, string[]>();
	const rootNames: string[] = [];
	for (let i = 0; i < roots; i++) rootNames.push(mk());
	let frontier = rootNames;
	for (let l = 1; l < levels; l++) {
		const nextLevel: string[] = [];
		for (const parent of frontier) {
			const kids: string[] = [];
			for (let f = 0; f < fanout; f++) kids.push(mk());
			deps.set(parent, kids);
			nextLevel.push(...kids);
		}
		frontier = nextLevel;
	}
	for (const leaf of frontier) deps.set(leaf, []);
	names.push(...Array.from({ length: n }, (_, i) => `@acme/p${i}`));
	return { deps, rootNames, total: n };
}

function seedFrom(roots: number, levels: number, fanout: number) {
	const { deps, rootNames, total } = buildGraph(roots, levels, fanout);
	const db = new DatabaseSync(':memory:');
	db.exec(`CREATE TABLE packages (id TEXT PRIMARY KEY, scope TEXT, name TEXT);
CREATE TABLE package_versions (id TEXT PRIMARY KEY, package_id TEXT, version TEXT, status TEXT,
  semver_major INT, semver_minor INT, semver_patch INT, prerelease TEXT, created_at INT,
  engine_range TEXT, manifest_json TEXT, tarball_sha256 TEXT, tar_manifest_json TEXT);
CREATE TABLE tombstones (package_id TEXT, version TEXT, PRIMARY KEY (package_id, version));`);
	const ins = db.prepare('INSERT INTO packages (id, scope, name) VALUES (?, ?, ?)');
	const insv = db.prepare(`INSERT INTO package_versions (id, package_id, version, status,
  semver_major, semver_minor, semver_patch, prerelease, created_at, engine_range, manifest_json, tarball_sha256, tar_manifest_json)
  VALUES (?, ?, ?, 'live', 1, 0, 0, '', 1, '', ?, 'sha', '[]')`);
	for (let i = 0; i < total; i++) {
		const full = `@acme/p${i}`;
		const id = `pkg_${full}`;
		ins.run(id, 'acme', `p${i}`);
		const kids = deps.get(full) ?? [];
		const manifest = kids.length ? JSON.stringify({ deps: Object.fromEntries(kids.map((k) => [k, '*'])) }) : '{}';
		insv.run(`ver_${id}_1.0.0`, id, '1.0.0', manifest);
	}
	return { db, requirements: Object.fromEntries(rootNames.map((r) => [r, '*'])), total };
}

void seed;

async function measure(label: string, roots: number, levels: number, fanout: number) {
	const { db, requirements, total } = seedFrom(roots, levels, fanout);
	const results: Record<string, { stmts: number; rt: number; rows: number; nodes: number; levels: number }> = {};
	for (const arm of ['old', 'new']) {
		const store = new Store(db);
		store.counter = { statements: 0, roundTrips: 0, rowsRead: 0 };
		const t0 = performance.now();
		const out = arm === 'old' ? await oldResolve(store, { ...requirements }) : await newResolve(store, { ...requirements });
		results[arm] = {
			stmts: store.counter.statements,
			rt: store.counter.roundTrips,
			rows: store.counter.rowsRead,
			nodes: out.nodes,
			levels: 'levels' in out ? out.levels : 0
		};
		void t0;
	}
	console.log(
		`${label.padEnd(28)} nodes=${String(total).padStart(4)}  ` +
			`old: rt=${String(results.old.rt).padStart(4)} stmts=${String(results.old.stmts).padStart(4)} rows=${String(results.old.rows).padStart(4)} levels=${results.old.levels}   |   ` +
			`new: rt=${String(results.new.rt).padStart(3)} stmts=${String(results.new.stmts).padStart(3)} rows=${String(results.new.rows).padStart(4)}`
	);
}

console.log('synthetic deeper graphs (local store, exact statement counts)\n');
await measure('17 roots, 1 level', 17, 1, 0);
await measure('200 roots, 1 level', 200, 1, 0);
await measure('10 roots, 3 levels x9', 10, 3, 9);
await measure('10 roots, 4 levels x3', 10, 4, 3);
await measure('10 roots, 4 levels x9 (1000)', 10, 4, 9);
await measure('10 roots, 5 levels x3 (~810)', 10, 5, 3);