import { createClient, type Client } from '@libsql/client/web';
import { maxSatisfying, parseSemver, levelize, satisfiesRange } from './registry.js';

// Temporary ownership model (no account system yet): a single org owner.
// `rovelstars` owns every scope published through the org token, and the
// `std` scope is reserved for the standard library. Per-user scopes arrive
// with the unified account system; the schema already carries owners.
export const ORG_OWNER = 'rovelstars';
export const RESERVED_SCOPES = ['std'];

export interface PackageSummary {
	name: string;
	description: string;
	author: string;
	repository: string;
	license: string;
	downloads: number;
	stars: number;
	tags: string[];
	updatedAt: number;
	latest: string;
	versionCount: number;
}

export interface VersionMeta {
	version: string;
	checksum: string;
	status: string;
	createdAt: number;
}

export interface PackageDetail extends PackageSummary {
	dependencies: string[];
	createdAt: number;
	versions: VersionMeta[];
}

export interface VersionDoc {
	version: string;
	readme: string;
	docJson: string;
}

export const BASE_SCHEMA = `
CREATE TABLE IF NOT EXISTS scopes (
    name TEXT PRIMARY KEY,
    owner TEXT NOT NULL,
    reserved INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS packages (
    id TEXT PRIMARY KEY,
    scope TEXT NOT NULL DEFAULT '',
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    author TEXT NOT NULL,
    repository TEXT,
    license TEXT DEFAULT 'MIT',
    downloads INTEGER DEFAULT 0,
    stars INTEGER DEFAULT 0,
    tags TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(scope, name)
);

CREATE TABLE IF NOT EXISTS package_versions (
    id TEXT PRIMARY KEY,
    package_id TEXT NOT NULL REFERENCES packages(id),
    version TEXT NOT NULL,
    readme_markdown TEXT NOT NULL,
    doc_json TEXT NOT NULL,
    checksum TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'live',
    created_at INTEGER NOT NULL,
    UNIQUE(package_id, version)
);

CREATE TABLE IF NOT EXISTS benchmark_runs (
    id INTEGER PRIMARY KEY,
    github_run_id INTEGER NOT NULL UNIQUE,
    commit_sha TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS tombstones (
    package_id TEXT NOT NULL REFERENCES packages(id),
    version TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    PRIMARY KEY (package_id, version)
);

CREATE TABLE IF NOT EXISTS audit_log (
    id INTEGER PRIMARY KEY,
    action TEXT NOT NULL,
    full_name TEXT NOT NULL,
    version TEXT NOT NULL DEFAULT '',
    details_json TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_name ON audit_log(full_name, created_at);

CREATE TABLE IF NOT EXISTS chunks (
    hash TEXT PRIMARY KEY,
    size_bytes INTEGER NOT NULL,
    bytes BLOB NOT NULL,
    first_seen_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS manifest_chunks (
    chunk_hash TEXT NOT NULL REFERENCES chunks(hash),
    version_id TEXT NOT NULL REFERENCES package_versions(id) ON DELETE CASCADE,
    ord INTEGER NOT NULL,
    PRIMARY KEY (chunk_hash, version_id)
);
CREATE INDEX IF NOT EXISTS idx_mc_version ON manifest_chunks(version_id);

CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    password_salt TEXT NOT NULL,
    password_params TEXT NOT NULL,
    is_admin INTEGER NOT NULL DEFAULT 0,
    disclaimer_ack INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    token_hash TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);

CREATE TABLE IF NOT EXISTS orgs (
    id INTEGER PRIMARY KEY,
    scope TEXT NOT NULL UNIQUE,
    owner_user_id INTEGER NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL
);
`;

const SCOPE_RE = /^[a-z0-9][a-z0-9-]{0,31}$/;
const NAME_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;

export interface ParsedName {
	scope: string;
	name: string;
	full: string;
}

export function parsePackageName(full: string): ParsedName | null {
	if (full.startsWith('@')) {
		const slash = full.indexOf('/');
		if (slash < 2 || slash === full.length - 1) return null;
		const scope = full.slice(1, slash);
		const name = full.slice(slash + 1);
		if (!SCOPE_RE.test(scope) || !NAME_RE.test(name)) return null;
		return { scope, name, full: `@${scope}/${name}` };
	}
	if (!NAME_RE.test(full)) return null;
	return { scope: '', name: full, full };
}

export function packageId(scope: string, name: string): string {
	return scope ? `pkg_@${scope}/${name}` : `pkg_${name}`;
}

let client: Client | null = null;
let schemaReady = false;

export function getClient(env: Record<string, string | undefined>): Client | null {
	if (client) return client;
	const url = env['TURSO_DATABASE_URL'];
	const token = env['TURSO_AUTH_TOKEN'];
	if (!url) return null;
	try {
		client = createClient({ url, authToken: token });
		return client;
	} catch {
		return null;
	}
}

export async function ensureSchema(db: Client): Promise<void> {
	// Isolates persist across requests: migrate once, then skip the
	// half-dozen setup round-trips on every call (Turso is single-region,
	// each statement is a transatlantic HTTPS request for most visitors).
	if (schemaReady) return;
	for (const stmt of BASE_SCHEMA.split(';')) {
		const sql = stmt.trim();
		if (sql) await db.execute(sql);
	}
	const cols = await db.execute('SELECT name FROM pragma_table_info(?)', ['packages']);
	const names = new Set(cols.rows.map((r) => String(r['name'])));
	if (!names.has('scope')) {
		await db.execute('ALTER TABLE packages ADD COLUMN scope TEXT NOT NULL DEFAULT ? ', ['']);
	}
	const vcols = await db.execute("SELECT name FROM pragma_table_info(?)", ['package_versions']);
	const vnames = new Set(vcols.rows.map((r) => String(r['name'])));
	if (!vnames.has('status')) {
		await db.execute("ALTER TABLE package_versions ADD COLUMN status TEXT NOT NULL DEFAULT 'live'");
	}
	const backfill: Array<[string, string]> = [
		['semver_major', 'INTEGER NOT NULL DEFAULT 0'],
		['semver_minor', 'INTEGER NOT NULL DEFAULT 0'],
		['semver_patch', 'INTEGER NOT NULL DEFAULT 0'],
		['prerelease', "TEXT NOT NULL DEFAULT ''"],
		['engine_range', "TEXT NOT NULL DEFAULT ''"],
		['manifest_json', "TEXT NOT NULL DEFAULT '{}'"],
		['guides_json', "TEXT NOT NULL DEFAULT '[]'"],
		['tarball_sha256', "TEXT NOT NULL DEFAULT ''"],
		['request_id', "TEXT NOT NULL DEFAULT ''"]
	];
	for (const [col, ddl] of backfill) {
		if (!vnames.has(col)) {
			await db.execute(`ALTER TABLE package_versions ADD COLUMN ${col} ${ddl}`);
		}
	}
	const acols = await db.execute('SELECT name FROM pragma_table_info(?)', ['audit_log']);
	const anames = new Set(acols.rows.map((r) => String(r['name'])));
	if (!anames.has('request_id')) {
		await db.execute("ALTER TABLE audit_log ADD COLUMN request_id TEXT NOT NULL DEFAULT ''");
	}
	schemaReady = true;
}

// Publish auth: exact match against the configured org token. When no
// token is configured (local dev / preview) only `preview-` tokens pass,
// so production misconfiguration fails closed instead of open.
export function checkPublishToken(
	env: Record<string, string | undefined>,
	provided: string
): boolean {
	const configured = env['PUBLISH_TOKEN'];
	if (configured) return provided === configured && provided.length > 0;
	return provided.startsWith('preview-') && provided.length > 8;
}

function toSummary(
	r: Record<string, unknown>,
	latest: string,
	versionCount: number
): PackageSummary {
	const scope = String(r['scope'] ?? '');
	const name = String(r['name']);
	return {
		name: scope ? `@${scope}/${name}` : name,
		description: String(r['description']),
		author: String(r['author']),
		repository: String(r['repository'] ?? ''),
		license: String(r['license'] ?? 'MIT'),
		downloads: Number(r['downloads'] ?? 0),
		stars: Number(r['stars'] ?? 0),
		tags: String(r['tags'] ?? '')
			.split(',')
			.map((t) => t.trim())
			.filter(Boolean),
		updatedAt: Number(r['updated_at'] ?? 0),
		latest,
		versionCount
	};
}

export async function listPackages(
	env: Record<string, string | undefined>
): Promise<PackageSummary[]> {
	const db = getClient(env);
	if (!db) return [];
	await ensureSchema(db);
	const rs = await db.execute(
		"SELECT p.*, (SELECT version FROM package_versions v WHERE v.package_id = p.id AND v.status = 'live' ORDER BY v.created_at DESC LIMIT 1) AS latest, (SELECT COUNT(*) FROM package_versions v WHERE v.package_id = p.id) AS version_count FROM packages p ORDER BY p.updated_at DESC"
	);
	return rs.rows.map((r) =>
		toSummary(
			r as Record<string, unknown>,
			String(r['latest'] ?? ''),
			Number(r['version_count'] ?? 0)
		)
	);
}

export async function getPackage(
	env: Record<string, string | undefined>,
	full: string
): Promise<PackageDetail | null> {
	const parsed = parsePackageName(full);
	const db = getClient(env);
	if (!parsed || !db) return null;
	await ensureSchema(db);
	const rs = await db.execute({
		sql: 'SELECT * FROM packages WHERE scope = ? AND name = ?',
		args: [parsed.scope, parsed.name]
	});
	const row = rs.rows[0] as Record<string, unknown> | undefined;
	if (!row) return null;
	const vs = await db.execute({
		sql: 'SELECT version, checksum, status, created_at FROM package_versions WHERE package_id = ? ORDER BY created_at DESC',
		args: [String(row['id'])]
	});
	const versions: VersionMeta[] = vs.rows.map((v) => ({
		version: String(v['version']),
		checksum: String(v['checksum']),
		status: String(v['status'] ?? 'live'),
		createdAt: Number(v['created_at'])
	}));
	if (versions.length === 0) return null;
	return {
		...toSummary(row, versions[0].version, versions.length),
		dependencies: [],
		createdAt: Number(row['created_at'] ?? 0),
		versions
	};
}

export async function getVersionDoc(
	env: Record<string, string | undefined>,
	full: string,
	version: string
): Promise<VersionDoc | null> {
	const parsed = parsePackageName(full);
	const db = getClient(env);
	if (!parsed || !db) return null;
	await ensureSchema(db);
	const rs = await db.execute({
		sql: `SELECT v.readme_markdown, v.doc_json FROM package_versions v
		      JOIN packages p ON p.id = v.package_id
		      WHERE p.scope = ? AND p.name = ? AND v.version = ?`,
		args: [parsed.scope, parsed.name, version]
	});
	const row = rs.rows[0];
	if (!row) return null;
	return {
		version,
		readme: String(row['readme_markdown']),
		docJson: String(row['doc_json'])
	};
}

export interface PublishPayload {
	name: string;
	version: string;
	description: string;
	author: string;
	license: string;
	tags: string[];
	readme: string;
	docJson: string;
	checksum: string;
	engineRange?: string;
	manifestJson?: string;
	guidesJson?: string;
	tarballSha256?: string;
	requestId?: string;
}

export interface PublishResult {
	name: string;
	version: string;
	created: boolean;
}

// Abuse guards (free-tier survival): hard caps enforced before any write.
// - Payload caps keep single publishes small.
// - MAX_VERSIONS_PER_WEEK bounds churn per package on a rolling window.
// - Tarball bytes (R2, when the upload endpoint lands) cap at 512 KiB.
export const MAX_DOC_JSON_BYTES = 1024 * 1024;
export const MAX_README_BYTES = 200 * 1024;
export const MAX_VERSIONS_PER_WEEK = 20;
export const MAX_TARBALL_BYTES = 512 * 1024;

export async function publishPackage(
	env: Record<string, string | undefined>,
	p: PublishPayload
): Promise<PublishResult> {
	const parsed = parsePackageName(p.name);
	const db = getClient(env);
	if (!parsed || !db) throw new Error('invalid package name or missing database');
	const sem = parseSemver(p.version);
	if (!sem) throw new Error('invalid package version (expected X.Y.Z)');
	const now = Math.floor(Date.now() / 1000);
	await ensureSchema(db);
	await db.execute({
		sql: `INSERT INTO scopes (name, owner, reserved, created_at) VALUES (?, ?, ?, ?)
		      ON CONFLICT(name) DO NOTHING`,
		args: [parsed.scope, ORG_OWNER, RESERVED_SCOPES.includes(parsed.scope) ? 1 : 0, now]
	});
	const id = packageId(parsed.scope, parsed.name);
	await db.execute({
		sql: `INSERT INTO packages (id, scope, name, description, author, repository, license, downloads, stars, tags, created_at, updated_at)
		      VALUES (?, ?, ?, ?, ?, '', ?, 0, 0, ?, ?, ?)
		      ON CONFLICT(scope, name) DO UPDATE SET description = excluded.description, updated_at = excluded.updated_at`,
		args: [id, parsed.scope, parsed.name, p.description, p.author, p.license, p.tags.join(','), now, now]
	});
	const res = await db.execute({
		sql: `INSERT INTO package_versions (id, package_id, version, readme_markdown, doc_json, checksum, status, created_at,
		      semver_major, semver_minor, semver_patch, prerelease,
		      engine_range, manifest_json, guides_json, tarball_sha256, request_id)
		      VALUES (?, ?, ?, ?, ?, ?, 'live', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
		      ON CONFLICT(package_id, version) DO NOTHING`,
		args: [
			`ver_${id}_${p.version}`,
			id,
			p.version,
			p.readme,
			p.docJson,
			p.checksum,
			now,
			sem.major,
			sem.minor,
			sem.patch,
			sem.prerelease,
			p.engineRange ?? '',
			p.manifestJson ?? '{}',
			p.guidesJson ?? '[]',
			p.tarballSha256 ?? '',
			p.requestId ?? ''
		]
	});
	const created = (res.rowsAffected ?? 0) > 0;
	if (created) {
		await db.execute({
			sql: `INSERT INTO audit_log (action, full_name, version, details_json, request_id, created_at)
			      VALUES ('publish', ?, ?, ?, ?, ?)`,
			args: [parsed.full, p.version, JSON.stringify({ checksum: p.checksum }), p.requestId ?? '', now]
		});
	}
	return { name: parsed.full, version: p.version, created };
}

export async function recentVersionCount(
	env: Record<string, string | undefined>,
	full: string,
	sinceSeconds: number
): Promise<number> {
	const parsed = parsePackageName(full);
	const db = getClient(env);
	if (!parsed || !db) return 0;
	await ensureSchema(db);
	const rs = await db.execute({
		sql: `SELECT COUNT(*) AS n FROM package_versions v
		      JOIN packages p ON p.id = v.package_id
		      WHERE p.scope = ? AND p.name = ? AND v.created_at > ?`,
		args: [parsed.scope, parsed.name, Math.floor(Date.now() / 1000) - sinceSeconds]
	});
	return Number(rs.rows[0]?.['n'] ?? 0);
}

export interface VersionRow {
	id: string;
	version: string;
	status: string;
	checksum: string;
	tarballSha256: string;
	engineRange: string;
	manifestJson: string;
	guidesJson: string;
	requestId: string;
	createdAt: number;
}

const VERSION_COLS = `id, package_id, version, readme_markdown, doc_json, checksum,
	tarball_sha256, engine_range, manifest_json, guides_json, request_id,
	semver_major, semver_minor, semver_patch, prerelease, status, created_at`;

function toVersionRow(r: Record<string, unknown>): VersionRow {
	return {
		id: String(r['id']),
		version: String(r['version']),
		status: String(r['status'] ?? 'live'),
		checksum: String(r['checksum'] ?? ''),
		tarballSha256: String(r['tarball_sha256'] ?? ''),
		engineRange: String(r['engine_range'] ?? ''),
		manifestJson: String(r['manifest_json'] ?? '{}'),
		guidesJson: String(r['guides_json'] ?? '[]'),
		requestId: String(r['request_id'] ?? ''),
		createdAt: Number(r['created_at'] ?? 0)
	};
}

const SEMVER_ORDER = `semver_major DESC, semver_minor DESC, semver_patch DESC,
	CASE WHEN prerelease = '' THEN 1 ELSE 0 END DESC, prerelease DESC, created_at DESC`;

export async function getVersionRow(
	env: Record<string, string | undefined>,
	full: string,
	version: string
): Promise<{ row: VersionRow; withdrawn: boolean } | null> {
	const parsed = parsePackageName(full);
	const db = getClient(env);
	if (!parsed || !db) return null;
	await ensureSchema(db);
	const rs = await db.execute({
		sql: `SELECT ${VERSION_COLS} FROM package_versions v
		      JOIN packages p ON p.id = v.package_id
		      WHERE p.scope = ? AND p.name = ? AND v.version = ?`,
		args: [parsed.scope, parsed.name, version]
	});
	const r = rs.rows[0] as Record<string, unknown> | undefined;
	if (!r) return null;
	const row = toVersionRow(r);
	const withdrawn = row.status === 'tombstoned' || (await isVersionWithdrawn(env, full, version));
	return { row, withdrawn };
}

export async function listLiveVersions(
	env: Record<string, string | undefined>,
	full: string
): Promise<string[]> {
	const parsed = parsePackageName(full);
	const db = getClient(env);
	if (!parsed || !db) return [];
	await ensureSchema(db);
	const rs = await db.execute({
		sql: `SELECT v.version FROM package_versions v
		      JOIN packages p ON p.id = v.package_id
		      WHERE p.scope = ? AND p.name = ? AND v.status IN ('live', 'yanked')
		      ORDER BY ${SEMVER_ORDER}`,
		args: [parsed.scope, parsed.name]
	});
	return rs.rows.map((r) => String(r['version']));
}

export async function getLatestVersion(
	env: Record<string, string | undefined>,
	full: string
): Promise<string | null> {
	const versions = await listLiveVersions(env, full);
	return versions[0] ?? null;
}

export async function yankVersion(
	env: Record<string, string | undefined>,
	full: string,
	version: string
): Promise<boolean> {
	const parsed = parsePackageName(full);
	const db = getClient(env);
	if (!parsed || !db) throw new Error('invalid package name or missing database');
	await ensureSchema(db);
	const id = packageId(parsed.scope, parsed.name);
	const res = await db.execute({
		sql: `UPDATE package_versions SET status = 'yanked'
		      WHERE package_id = ? AND version = ? AND status = 'live'`,
		args: [id, version]
	});
	if ((res.rowsAffected ?? 0) === 0) return false;
	await db.execute({
		sql: `INSERT INTO audit_log (action, full_name, version, created_at)
		      VALUES ('yank', ?, ?, ?)`,
		args: [parsed.full, version, Math.floor(Date.now() / 1000)]
	});
	return true;
}

export async function transferScope(
	env: Record<string, string | undefined>,
	scope: string,
	newOwner: string,
	force: boolean
): Promise<void> {
	const db = getClient(env);
	if (!db) throw new Error('missing database');
	await ensureSchema(db);
	if (!force) {
		const live = await db.execute({
			sql: `SELECT COUNT(*) AS n FROM package_versions v
			      JOIN packages p ON p.id = v.package_id
			      WHERE p.scope = ? AND v.status IN ('live', 'yanked')`,
			args: [scope]
		});
		if (Number(live.rows[0]?.['n'] ?? 0) > 0) {
			throw new Error('scope has live versions (retry with force)');
		}
	}
	await db.execute({
		sql: 'UPDATE packages SET scope = ?, updated_at = ? WHERE scope = ?',
		args: [`${newOwner}`, Math.floor(Date.now() / 1000), scope]
	});
	await db.execute({
		sql: `INSERT INTO audit_log (action, full_name, details_json, created_at)
		      VALUES ('transfer', ?, ?, ?)`,
		args: [`@${scope}`, JSON.stringify({ newOwner, force }), Math.floor(Date.now() / 1000)]
	});
}

export async function putChunk(
	env: Record<string, string | undefined>,
	hash: string,
	sizeBytes: number,
	bytes: Uint8Array
): Promise<void> {
	const db = getClient(env);
	if (!db) throw new Error('missing database');
	await ensureSchema(db);
	await db.execute({
		sql: `INSERT INTO chunks (hash, size_bytes, bytes, first_seen_at)
		      VALUES (?, ?, ?, ?)
		      ON CONFLICT(hash) DO NOTHING`,
		args: [hash, sizeBytes, bytes, Math.floor(Date.now() / 1000)]
	});
}

export async function linkVersionChunks(
	env: Record<string, string | undefined>,
	versionRowId: string,
	hashes: string[]
): Promise<void> {
	const db = getClient(env);
	if (!db) throw new Error('missing database');
	await ensureSchema(db);
	let ord = 0;
	for (const hash of hashes) {
		await db.execute({
			sql: `INSERT INTO manifest_chunks (chunk_hash, version_id, ord)
			      VALUES (?, ?, ?)
			      ON CONFLICT(chunk_hash, version_id) DO NOTHING`,
			args: [hash, versionRowId, ord++]
		});
	}
}

export async function getVersionBytes(
	env: Record<string, string | undefined>,
	versionRowId: string
): Promise<Uint8Array | null> {
	const db = getClient(env);
	if (!db) return null;
	await ensureSchema(db);
	const rs = await db.execute({
		sql: `SELECT c.bytes AS bytes FROM manifest_chunks mc
		      JOIN chunks c ON c.hash = mc.chunk_hash
		      WHERE mc.version_id = ? ORDER BY mc.ord ASC`,
		args: [versionRowId]
	});
	if (rs.rows.length === 0) return null;
	const parts: Uint8Array[] = rs.rows.map((r) => {
		const b = r['bytes'] as Uint8Array | ArrayBuffer;
		return b instanceof Uint8Array ? b : new Uint8Array(b);
	});
	const total = parts.reduce((n, p) => n + p.length, 0);
	const out = new Uint8Array(total);
	let off = 0;
	for (const p of parts) {
		out.set(p, off);
		off += p.length;
	}
	return out;
}

export async function findOrphanChunks(
	env: Record<string, string | undefined>,
	beforeUnix: number,
	limit: number
): Promise<string[]> {
	const db = getClient(env);
	if (!db) return [];
	await ensureSchema(db);
	const rs = await db.execute({
		sql: `SELECT c.hash AS hash FROM chunks c
		      WHERE c.first_seen_at < ?
		      AND NOT EXISTS (
		        SELECT 1 FROM manifest_chunks mc
		        JOIN package_versions v ON v.id = mc.version_id
		        WHERE mc.chunk_hash = c.hash AND v.status IN ('live', 'yanked'))
		      LIMIT ?`,
		args: [beforeUnix, Math.max(1, Math.min(1000, limit))]
	});
	return rs.rows.map((r) => String(r['hash']));
}

export async function reverifyChunks(
	env: Record<string, string | undefined>,
	hashes: string[]
): Promise<Set<string>> {
	const db = getClient(env);
	if (!db || hashes.length === 0) return new Set();
	await ensureSchema(db);
	const live = new Set<string>();
	for (const batch of chunked(hashes, 50)) {
		const marks = batch.map(() => '?').join(',');
		const rs = await db.execute({
			sql: `SELECT mc.chunk_hash AS hash FROM manifest_chunks mc
			      JOIN package_versions v ON v.id = mc.version_id
			      WHERE mc.chunk_hash IN (${marks}) AND v.status IN ('live', 'yanked')
			      GROUP BY mc.chunk_hash`,
			args: [...batch]
		});
		for (const r of rs.rows) live.add(String(r['hash']));
	}
	return live;
}

function chunked<T>(xs: T[], n: number): T[][] {
	const out: T[][] = [];
	for (let i = 0; i < xs.length; i += n) out.push(xs.slice(i, i + n));
	return out;
}

export async function deleteChunks(
	env: Record<string, string | undefined>,
	hashes: string[]
): Promise<number> {
	const db = getClient(env);
	if (!db || hashes.length === 0) return 0;
	await ensureSchema(db);
	let deleted = 0;
	for (const batch of chunked(hashes, 50)) {
		const marks = batch.map(() => '?').join(',');
		const res = await db.execute({
			sql: `DELETE FROM chunks WHERE hash IN (${marks})`,
			args: [...batch]
		});
		deleted += res.rowsAffected ?? 0;
	}
	await db.execute({
		sql: `INSERT INTO audit_log (action, full_name, details_json, created_at)
		      VALUES ('gc-sweep', '', ?, ?)`,
		args: [JSON.stringify({ deleted: hashes }), Math.floor(Date.now() / 1000)]
	});
	return deleted;
}

export interface ResolveNode {
	full: string;
	version: string;
	path: string;
	integrity: string;
	engineRange: string;
	deps: string[];
	yanked: boolean;
}

export interface HaveEntry {
	full: string;
	version: string;
	integrity: string;
}

export async function resolveGraph(
	env: Record<string, string | undefined>,
	requirements: Record<string, string>,
	have: HaveEntry[]
): Promise<{ resolved: Record<string, string>; levels: ResolveNode[][] }> {
	const held = new Map(have.map((h) => [`${h.full}@${h.version}`, h.integrity]));
	const memo = new Map<string, ResolveNode>();
	const visiting: string[] = [];
	const resolved: Record<string, string> = {};

	const visit = async (full: string, range: string): Promise<string> => {
		if (resolved[full]) {
			if (!satisfiesRange(resolved[full], range)) {
				throw new Error(`conflicting ranges for ${full}: ${resolved[full]} does not satisfy ${range}`);
			}
			return resolved[full];
		}
		const versions = await listLiveVersions(env, full);
		const pick = maxSatisfying(versions, range);
		if (!pick) {
			throw new Error(
				versions.length === 0
					? `package ${full} does not exist`
					: `no version of ${full} satisfies ${range} (have ${versions.slice(0, 5).join(', ')})`
			);
		}
		resolved[full] = pick;
		const id = `${full}@${pick}`;
		if (memo.has(id)) return pick;
		if (visiting.includes(id)) {
			throw new Error(`dependency cycle: ${[...visiting, id].join(' -> ')}`);
		}
		visiting.push(id);
		try {
			const found = await getVersionRow(env, full, pick);
			if (!found) throw new Error(`package ${id} does not exist`);
			const { row } = found;
			const key = `${full}@${pick}`;
			if (held.get(key) === row.tarballSha256 && row.tarballSha256 !== '') {
				visiting.pop();
				return pick;
			}
			let deps: Record<string, string> = {};
			try {
				const manifest = JSON.parse(row.manifestJson) as Record<string, unknown>;
				if (manifest['deps'] && typeof manifest['deps'] === 'object') {
					deps = manifest['deps'] as Record<string, string>;
				}
			} catch {
				deps = {};
			}
			const depIds: string[] = [];
			for (const [name, depRange] of Object.entries(deps)) {
				const depVersion = await visit(name, String(depRange));
				depIds.push(`${name}@${depVersion}`);
			}
			memo.set(id, {
				full,
				version: pick,
				path: `${full}/${pick}/download`,
				integrity: row.tarballSha256,
				engineRange: row.engineRange,
				deps: depIds.map((d) => d.split('@').slice(0, -1).join('@')),
				yanked: row.status === 'yanked'
			});
		} finally {
			visiting.pop();
		}
		return pick;
	};

	for (const [full, range] of Object.entries(requirements)) {
		if (!parsePackageName(full)) throw new Error(`invalid package name: ${full}`);
	}
	for (const [full, range] of Object.entries(requirements)) {
		await visit(full, String(range));
	}
	const nodes = [...memo.values()];
	const order = levelize(nodes.map((n) => ({ id: `${n.full}@${n.version}`, deps: n.deps.map((d) => {
		const v = resolved[d];
		return v ? `${d}@${v}` : d;
	}) })));
	const byId = new Map(nodes.map((n) => [`${n.full}@${n.version}`, n]));
	return { resolved, levels: order.map((ids) => ids.map((id) => byId.get(id)!)) };
}

export interface BenchmarkRun {
	id: number;
	githubRunId: number;
	commitSha: string;
	snapshotJson: string;
	createdAt: number;
}

export const MAX_SNAPSHOT_BYTES = 1024 * 1024;

export async function insertBenchmarkRun(
	env: Record<string, string | undefined>,
	githubRunId: number,
	commitSha: string,
	snapshotJson: string
): Promise<BenchmarkRun> {
	const db = getClient(env);
	if (!db) throw new Error('missing database');
	await ensureSchema(db);
	const now = Math.floor(Date.now() / 1000);
	const res = await db.execute({
		sql: `INSERT INTO benchmark_runs (github_run_id, commit_sha, snapshot_json, created_at)
		      VALUES (?, ?, ?, ?)`,
		args: [githubRunId, commitSha, snapshotJson, now]
	});
	return {
		id: Number(res.lastInsertRowid ?? 0),
		githubRunId,
		commitSha,
		snapshotJson,
		createdAt: now
	};
}

export async function getLatestBenchmarks(
	env: Record<string, string | undefined>
): Promise<BenchmarkRun | null> {
	const db = getClient(env);
	if (!db) return null;
	await ensureSchema(db);
	const rs = await db.execute(
		'SELECT id, github_run_id, commit_sha, snapshot_json, created_at FROM benchmark_runs ORDER BY id DESC LIMIT 1'
	);
	const row = rs.rows[0] as Record<string, unknown> | undefined;
	if (!row) return null;
	return {
		id: Number(row['id']),
		githubRunId: Number(row['github_run_id']),
		commitSha: String(row['commit_sha']),
		snapshotJson: String(row['snapshot_json']),
		createdAt: Number(row['created_at'])
	};
}

export async function listBenchmarkRuns(
	env: Record<string, string | undefined>,
	limit: number
): Promise<Omit<BenchmarkRun, 'snapshotJson'>[]> {
	const db = getClient(env);
	if (!db) return [];
	await ensureSchema(db);
	const rs = await db.execute({
		sql: 'SELECT id, github_run_id, commit_sha, created_at FROM benchmark_runs ORDER BY id DESC LIMIT ?',
		args: [Math.max(1, Math.min(60, Math.floor(limit) || 12))]
	});
	return rs.rows.map((r) => ({
		id: Number(r['id']),
		githubRunId: Number(r['github_run_id']),
		commitSha: String(r['commit_sha']),
		createdAt: Number(r['created_at'])
	}));
}

export async function isVersionWithdrawn(
	env: Record<string, string | undefined>,
	full: string,
	version: string
): Promise<boolean> {
	const parsed = parsePackageName(full);
	const db = getClient(env);
	if (!parsed || !db) return false;
	await ensureSchema(db);
	const rs = await db.execute({
		sql: `SELECT 1 AS n FROM tombstones t
		      JOIN packages p ON p.id = t.package_id
		      WHERE p.scope = ? AND p.name = ? AND t.version = ?`,
		args: [parsed.scope, parsed.name, version]
	});
	return rs.rows.length > 0;
}

export async function versionReuseBlocked(
	env: Record<string, string | undefined>,
	full: string,
	version: string
): Promise<boolean> {
	const parsed = parsePackageName(full);
	const db = getClient(env);
	if (!parsed || !db) return false;
	await ensureSchema(db);
	const id = packageId(parsed.scope, parsed.name);
	const live = await db.execute({
		sql: 'SELECT 1 AS n FROM package_versions WHERE package_id = ? AND version = ?',
		args: [id, version]
	});
	if (live.rows.length > 0) return true;
	if (await isVersionWithdrawn(env, full, version)) return true;
	const audit = await db.execute({
		sql: `SELECT 1 AS n FROM audit_log
		      WHERE full_name = ? AND version = ?
		      AND action IN ('takedown', 'transfer', 'special-delete', 'publish')`,
		args: [parsed.full, version]
	});
	return audit.rows.length > 0;
}

export async function recordTombstone(
	env: Record<string, string | undefined>,
	full: string,
	version: string,
	reason: string
): Promise<void> {
	const parsed = parsePackageName(full);
	const db = getClient(env);
	if (!parsed || !db) throw new Error('invalid package name or missing database');
	const now = Math.floor(Date.now() / 1000);
	await ensureSchema(db);
	const id = packageId(parsed.scope, parsed.name);
	await db.execute({
		sql: `INSERT INTO tombstones (package_id, version, reason, created_at)
		      VALUES (?, ?, ?, ?)
		      ON CONFLICT(package_id, version) DO NOTHING`,
		args: [id, version, reason, now]
	});
	await db.execute({
		sql: `UPDATE package_versions SET status = 'tombstoned'
		      WHERE package_id = ? AND version = ?`,
		args: [id, version]
	});
	await db.execute({
		sql: `INSERT INTO audit_log (action, full_name, version, details_json, created_at)
		      VALUES ('takedown', ?, ?, ?, ?)`,
		args: [parsed.full, version, JSON.stringify({ reason }), now]
	});
}
