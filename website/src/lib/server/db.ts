import { createClient, type Client } from '@libsql/client/web';
import { maxSatisfying, parseSemver, levelize, satisfiesRange, selectLatestVersion } from './registry.js';
import { sha256Hex, type TarEntryRef } from './chunks.js';
import { r2Bucket, r2PutIfMissing, r2Get } from './r2.js';
import { freshQuotaState, shouldSample, effectiveUsage, quotaBudgetBytes, quotaGuardEnabled } from './quota.js';

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
	keywords: string[];
	updatedAt: number;
	latest: string;
	versionCount: number;
	dependents: number;
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
	guides: string;
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
    owner TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL,
    author TEXT NOT NULL,
    repository TEXT,
    license TEXT DEFAULT 'MIT',
    downloads INTEGER DEFAULT 0,
    stars INTEGER DEFAULT 0,
    tags TEXT NOT NULL,
    keywords TEXT DEFAULT '',
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
    PRIMARY KEY (version_id, ord)
);
CREATE INDEX IF NOT EXISTS idx_mc_hash ON manifest_chunks(chunk_hash);

CREATE TABLE IF NOT EXISTS package_deps (
    package_id TEXT NOT NULL REFERENCES packages(id),
    dep_name TEXT NOT NULL,
    PRIMARY KEY (package_id, dep_name)
);
CREATE INDEX IF NOT EXISTS idx_deps_name ON package_deps(dep_name);

CREATE TABLE IF NOT EXISTS download_daily (
    package_id TEXT NOT NULL REFERENCES packages(id),
    day TEXT NOT NULL,
    downloads INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (package_id, day)
);

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

CREATE TABLE IF NOT EXISTS auth_attempts (
    ip TEXT NOT NULL,
    username TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    window_start INTEGER NOT NULL,
    PRIMARY KEY (ip, username)
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

export const KEYWORD_RE = /^[a-z0-9-]+$/;
export const MAX_KEYWORDS = 12;
export const MAX_KEYWORD_LEN = 64;

export function validateKeywords(v: unknown): string[] | null {
	if (v === undefined) return [];
	if (!Array.isArray(v) || v.length > MAX_KEYWORDS) return null;
	const out: string[] = [];
	for (const k of v) {
		if (typeof k !== 'string' || k.length === 0 || k.length > MAX_KEYWORD_LEN) return null;
		if (!KEYWORD_RE.test(k)) return null;
		if (!out.includes(k)) out.push(k);
	}
	return out;
}

const NON_REGISTRY_VALUE_RE = /^(?:\.|\/|file:|path:|git[+:]|https?:|native:|url:)|:\/\//i;

export function extractDepNames(manifestJson: string): string[] {
	let manifest: Record<string, unknown>;
	try {
		manifest = JSON.parse(manifestJson) as Record<string, unknown>;
	} catch {
		return [];
	}
	const deps = manifest['deps'];
	if (!deps || typeof deps !== 'object' || Array.isArray(deps)) return [];
	const out: string[] = [];
	for (const [key, value] of Object.entries(deps as Record<string, unknown>)) {
		if (!parsePackageName(key)) continue;
		if (typeof value === 'string' && NON_REGISTRY_VALUE_RE.test(value)) continue;
		if (!out.includes(key)) out.push(key);
	}
	return out.sort();
}

export interface PackageFilter {
	search?: string;
	keyword?: string;
	license?: string;
	minDownloads?: number;
	updatedSinceDays?: number;
}

export function matchesPackageFilter(
	p: PackageSummary,
	f: PackageFilter,
	nowSec: number = Math.floor(Date.now() / 1000)
): boolean {
	if (f.search) {
		const q = f.search.toLowerCase();
		if (!p.name.toLowerCase().includes(q) && !p.description.toLowerCase().includes(q)) return false;
	}
	if (f.keyword && !p.keywords.includes(f.keyword)) return false;
	if (f.license && p.license !== f.license) return false;
	if (f.minDownloads !== undefined && p.downloads < f.minDownloads) return false;
	if (f.updatedSinceDays !== undefined && p.updatedAt < nowSec - f.updatedSinceDays * 86400) return false;
	return true;
}

export function dayString(tsSec: number): string {
	return new Date(tsSec * 1000).toISOString().slice(0, 10);
}

let client: Client | null = null;
let schemaReady = false;
let schemaPromise: Promise<void> | null = null;

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
	if (schemaReady) return;
	if (schemaPromise) return schemaPromise;
	schemaPromise = migrateSchema(db).then(
		() => {
			schemaReady = true;
		},
		(e) => {
			schemaPromise = null;
			throw e;
		}
	);
	return schemaPromise;
}

// Turso serves SQLite over HTTP with no interactive transactions, so every
// multi-statement mutation below runs as one db.batch call: single round
// trip, applied atomically. That is the whole of our per-package
// serialization story, not BEGIN IMMEDIATE.
async function migrateSchema(db: Client): Promise<void> {
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
		['tar_manifest_json', "TEXT NOT NULL DEFAULT '[]'"],
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
	if (!names.has('owner')) {
		await db.execute("ALTER TABLE packages ADD COLUMN owner TEXT NOT NULL DEFAULT ''");
	}
	if (!names.has('keywords')) {
		await db.execute("ALTER TABLE packages ADD COLUMN keywords TEXT DEFAULT ''");
	}
	await db.execute("UPDATE packages SET owner = 'org' WHERE owner = ''");
	const mdef = await db.execute("SELECT sql FROM sqlite_master WHERE name = 'manifest_chunks'");
	if (String(mdef.rows[0]?.['sql'] ?? '').includes('PRIMARY KEY (chunk_hash, version_id)')) {
		await db.execute('ALTER TABLE manifest_chunks RENAME TO manifest_chunks_legacy');
		await db.execute(`CREATE TABLE manifest_chunks (
		    chunk_hash TEXT NOT NULL REFERENCES chunks(hash),
		    version_id TEXT NOT NULL REFERENCES package_versions(id) ON DELETE CASCADE,
		    ord INTEGER NOT NULL,
		    PRIMARY KEY (version_id, ord)
		)`);
		await db.execute(`INSERT INTO manifest_chunks (chunk_hash, version_id, ord)
		    SELECT chunk_hash, version_id, ROW_NUMBER() OVER (PARTITION BY version_id ORDER BY ord) - 1
		    FROM manifest_chunks_legacy`);
		await db.execute('DROP TABLE manifest_chunks_legacy');
		await db.execute('CREATE INDEX IF NOT EXISTS idx_mc_hash ON manifest_chunks(chunk_hash)');
	}
}

let previewTokenWarned = false;

// Publish auth: exact match against the configured org token. When PUBLISH_TOKEN
// is unset this accepts any self-minted `preview-...` string, so mutation is
// open to anyone who can reach the endpoint. That fail-open is deliberate for
// local dev and is a hole in any deployment that forgets the var, which is why
// the fallback warns instead of staying silent.
export function checkPublishToken(
	env: Record<string, string | undefined>,
	provided: string
): boolean {
	const configured = env['PUBLISH_TOKEN'];
	if (configured) return provided === configured && provided.length > 0;
	if (!previewTokenWarned) {
		previewTokenWarned = true;
		console.warn('PUBLISH_TOKEN is unset: any preview- token can publish. Set PUBLISH_TOKEN outside local dev.');
	}
	return provided.startsWith('preview-') && provided.length > 8;
}

function toSummary(
	r: Record<string, unknown>,
	latest: string,
	versionCount: number,
	dependents: number = 0
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
		keywords: String(r['keywords'] ?? '')
			.split(',')
			.map((t) => t.trim())
			.filter(Boolean),
		updatedAt: Number(r['updated_at'] ?? 0),
		latest,
		versionCount,
		dependents
	};
}

export async function listPackages(
	env: Record<string, string | undefined>,
	filters: PackageFilter = {}
): Promise<PackageSummary[]> {
	const db = getClient(env);
	if (!db) return [];
	await ensureSchema(db);
	const now = Math.floor(Date.now() / 1000);
	const rs = await db.execute('SELECT * FROM packages ORDER BY updated_at DESC');
	const vs = await db.execute('SELECT package_id, version, status FROM package_versions');
	const byPkg = new Map<string, Array<{ version: string; status: string }>>();
	for (const v of vs.rows) {
		const id = String(v['package_id']);
		const list = byPkg.get(id) ?? [];
		list.push({ version: String(v['version']), status: String(v['status'] ?? 'live') });
		byPkg.set(id, list);
	}
	let sums = new Map<string, number>();
	try {
		const ds = await db.execute({
			sql: 'SELECT package_id, SUM(downloads) AS n FROM download_daily WHERE day >= ? GROUP BY package_id',
			args: [dayString(now - 30 * 86400)]
		});
		for (const r of ds.rows) sums.set(String(r['package_id']), Number(r['n'] ?? 0));
	} catch {
		sums = new Map();
	}
	let depCounts = new Map<string, number>();
	try {
		const dc = await db.execute('SELECT dep_name, COUNT(*) AS n FROM package_deps GROUP BY dep_name');
		for (const r of dc.rows) depCounts.set(String(r['dep_name']), Number(r['n'] ?? 0));
	} catch {
		depCounts = new Map();
	}
	const out: PackageSummary[] = [];
	for (const r of rs.rows) {
		const rec = r as Record<string, unknown>;
		const id = String(rec['id']);
		const versions = byPkg.get(id) ?? [];
		const summary = toSummary(
			rec,
			selectLatestVersion(versions) ?? '',
			versions.length,
			0
		);
		summary.downloads = sums.get(id) ?? 0;
		summary.dependents = depCounts.get(summary.name) ?? 0;
		if (!matchesPackageFilter(summary, filters, now)) continue;
		out.push(summary);
	}
	return out;
}

export async function getPackageOwner(
	env: Record<string, string | undefined>,
	scope: string,
	name: string
): Promise<string | null> {
	const db = getClient(env);
	if (!db) return null;
	await ensureSchema(db);
	const rs = await db.execute({
		sql: 'SELECT owner FROM packages WHERE scope = ? AND name = ?',
		args: [scope, name]
	});
	const row = rs.rows[0] as Record<string, unknown> | undefined;
	return row ? String(row['owner'] ?? '') : null;
}

export async function getDependents(
	env: Record<string, string | undefined>,
	full: string
): Promise<string[]> {
	const parsed = parsePackageName(full);
	const db = getClient(env);
	if (!parsed || !db) return [];
	await ensureSchema(db);
	const rs = await db.execute({
		sql: `SELECT p.scope AS scope, p.name AS name FROM package_deps d
		      JOIN packages p ON p.id = d.package_id
		      WHERE d.dep_name = ? ORDER BY p.scope ASC, p.name ASC`,
		args: [parsed.full]
	});
	return rs.rows.map((r) => {
		const scope = String(r['scope'] ?? '');
		const name = String(r['name']);
		return scope ? `@${scope}/${name}` : name;
	});
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
	const depRs = await db.execute({
		sql: 'SELECT COUNT(*) AS n FROM package_deps WHERE dep_name = ?',
		args: [parsed.scope ? `@${parsed.scope}/${parsed.name}` : parsed.name]
	});
	return {
		...toSummary(
			row,
			selectLatestVersion(versions) ?? versions[0].version,
			versions.length,
			Number(depRs.rows[0]?.['n'] ?? 0)
		),
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
		sql: `SELECT v.readme_markdown, v.doc_json, v.guides_json FROM package_versions v
		      JOIN packages p ON p.id = v.package_id
		      WHERE p.scope = ? AND p.name = ? AND v.version = ?`,
		args: [parsed.scope, parsed.name, version]
	});
	const row = rs.rows[0];
	if (!row) return null;
	return {
		version,
		readme: String(row['readme_markdown']),
		docJson: String(row['doc_json']),
		guides: String(row['guides_json'] ?? '[]')
	};
}

export interface PublishPayload {
	name: string;
	version: string;
	description: string;
	author: string;
	license: string;
	tags: string[];
	keywords?: string[];
	readme: string;
	docJson: string;
	checksum: string;
	engineRange?: string;
	manifestJson?: string;
	guidesJson?: string;
	tarballSha256?: string;
	requestId?: string;
	owner?: string;
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
	const id = packageId(parsed.scope, parsed.name);
	const auditJson = JSON.stringify({ checksum: p.checksum });
	const keywords = (p.keywords ?? []).join(',');
	// Dependents reflect ever-depending packages (npm shows all): rows are
	// derived from manifests at publish time and are never deleted on
	// yank or tombstone, so no counter can desync under retries.
	const depNames = extractDepNames(p.manifestJson ?? '{}');
	// changes() reads the version insert directly above, so the audit row
	// lands only when this publish actually minted the version. The dep
	// inserts trail the audit row to keep that reading intact.
	const applied = await db.batch([
		{
			sql: `INSERT INTO scopes (name, owner, reserved, created_at) VALUES (?, ?, ?, ?)
			      ON CONFLICT(name) DO NOTHING`,
			args: [parsed.scope, ORG_OWNER, RESERVED_SCOPES.includes(parsed.scope) ? 1 : 0, now]
		},
		{
			sql: `INSERT INTO packages (id, scope, name, owner, description, author, repository, license, downloads, stars, tags, keywords, created_at, updated_at)
			      VALUES (?, ?, ?, ?, ?, ?, '', ?, 0, 0, ?, ?, ?, ?)
			      ON CONFLICT(scope, name) DO UPDATE SET description = excluded.description, keywords = excluded.keywords, updated_at = excluded.updated_at`,
			args: [id, parsed.scope, parsed.name, p.owner ?? '', p.description, p.author, p.license, p.tags.join(','), keywords, now, now]
		},
		{
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
		},
		{
			sql: `INSERT INTO audit_log (action, full_name, version, details_json, request_id, created_at)
			      SELECT 'publish', ?, ?, ?, ?, ? WHERE changes() > 0`,
			args: [parsed.full, p.version, auditJson, p.requestId ?? '', now]
		},
		...depNames.map((dep) => ({
			sql: `INSERT INTO package_deps (package_id, dep_name) VALUES (?, ?)
			      ON CONFLICT(package_id, dep_name) DO NOTHING`,
			args: [id, dep]
		}))
	]);
	const created = ((applied[2]?.rowsAffected ?? 0) > 0);
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
	docJson: string;
	manifestJson: string;
	guidesJson: string;
	tarManifestJson: string;
	requestId: string;
	createdAt: number;
}

const VERSION_COLS = `v.id AS id, v.package_id AS package_id, v.version AS version,
	v.readme_markdown AS readme_markdown, v.doc_json AS doc_json, v.checksum AS checksum,
	v.tarball_sha256 AS tarball_sha256, v.tar_manifest_json AS tar_manifest_json, v.engine_range AS engine_range,
	v.manifest_json AS manifest_json, v.guides_json AS guides_json, v.request_id AS request_id,
	v.semver_major AS semver_major, v.semver_minor AS semver_minor, v.semver_patch AS semver_patch,
	v.prerelease AS prerelease, v.status AS status, v.created_at AS created_at`;

function toVersionRow(r: Record<string, unknown>): VersionRow {
	return {
		id: String(r['id']),
		version: String(r['version']),
		status: String(r['status'] ?? 'live'),
		checksum: String(r['checksum'] ?? ''),
		tarballSha256: String(r['tarball_sha256'] ?? ''),
	tarManifestJson: String(r['tar_manifest_json'] ?? '[]'),
		engineRange: String(r['engine_range'] ?? ''),
		docJson: String(r['doc_json'] ?? '{"modules":[]}'),
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
	const parsed = parsePackageName(full);
	const db = getClient(env);
	if (!parsed || !db) return null;
	await ensureSchema(db);
	const rs = await db.execute({
		sql: `SELECT v.version AS version, v.status AS status FROM package_versions v
		      JOIN packages p ON p.id = v.package_id
		      WHERE p.scope = ? AND p.name = ?`,
		args: [parsed.scope, parsed.name]
	});
	return selectLatestVersion(
		rs.rows.map((r) => ({ version: String(r['version']), status: String(r['status'] ?? 'live') }))
	);
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
	const now = Math.floor(Date.now() / 1000);
	const applied = await db.batch([
		{
			sql: `UPDATE package_versions SET status = 'yanked'
			      WHERE package_id = ? AND version = ? AND status = 'live'`,
			args: [id, version]
		},
		{
			sql: `INSERT INTO audit_log (action, full_name, version, created_at)
			      SELECT 'yank', ?, ?, ? WHERE changes() > 0`,
			args: [parsed.full, version, now]
		}
	]);
	return ((applied[0]?.rowsAffected ?? 0) > 0);
}

export interface TransferAuditEntry {
	action: string;
	fullName: string;
	version: string;
	detailsJson: string;
}

export function buildTransferAuditEntries(
	oldFull: string,
	newFull: string,
	versions: string[]
): TransferAuditEntry[] {
	const out: TransferAuditEntry[] = [];
	for (const version of versions) {
		const detailsJson = JSON.stringify({ from: oldFull, to: newFull });
		out.push({ action: 'transfer', fullName: oldFull, version, detailsJson });
		if (newFull !== oldFull) out.push({ action: 'transfer', fullName: newFull, version, detailsJson });
	}
	return out;
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
	const now = Math.floor(Date.now() / 1000);
	const pkgs = await db.execute({
		sql: 'SELECT id, name FROM packages WHERE scope = ?',
		args: [scope]
	});
	const renames = pkgs.rows.map((r) => ({
		oldId: String(r['id']),
		name: String(r['name']),
		newId: packageId(newOwner, String(r['name']))
	}));
	const vrows = renames.length
		? await db.execute({
				sql: `SELECT package_id, version FROM package_versions WHERE package_id IN (${renames.map(() => '?').join(',')})`,
				args: renames.map((r) => r.oldId)
			})
		: { rows: [] as unknown[] };
	const byPkg = new Map<string, string[]>();
	for (const v of vrows.rows as Record<string, unknown>[]) {
		const list = byPkg.get(String(v['package_id'])) ?? [];
		list.push(String(v['version']));
		byPkg.set(String(v['package_id']), list);
	}
	const stmts: Array<{ sql: string; args: Array<string | number> }> = [];
	// Child rows migrate before the parent PK so immediate FK checks never
	// see a dangling reference mid-batch.
	for (const r of renames) {
		stmts.push({
			sql: 'UPDATE package_versions SET package_id = ? WHERE package_id = ?',
			args: [r.newId, r.oldId]
		});
		stmts.push({
			sql: 'UPDATE tombstones SET package_id = ? WHERE package_id = ?',
			args: [r.newId, r.oldId]
		});
		stmts.push({
			sql: 'UPDATE package_deps SET package_id = ? WHERE package_id = ?',
			args: [r.newId, r.oldId]
		});
		stmts.push({
			sql: 'UPDATE download_daily SET package_id = ? WHERE package_id = ?',
			args: [r.newId, r.oldId]
		});
	}
	for (const r of renames) {
		stmts.push({
			sql: 'UPDATE packages SET id = ?, scope = ?, updated_at = ? WHERE id = ?',
			args: [r.newId, newOwner, now, r.oldId]
		});
	}
	stmts.push({
		sql: `INSERT INTO scopes (name, owner, reserved, created_at) VALUES (?, ?, 0, ?)
		      ON CONFLICT(name) DO NOTHING`,
		args: [newOwner, newOwner, now]
	});
	stmts.push({ sql: 'DELETE FROM scopes WHERE name = ?', args: [scope] });
	stmts.push({ sql: 'DELETE FROM orgs WHERE scope = ?', args: [scope] });
	for (const r of renames) {
		const oldFull = scope ? `@${scope}/${r.name}` : r.name;
		const newFull = `@${newOwner}/${r.name}`;
		for (const e of buildTransferAuditEntries(oldFull, newFull, byPkg.get(r.oldId) ?? [])) {
			stmts.push({
				sql: `INSERT INTO audit_log (action, full_name, version, details_json, created_at)
				      VALUES (?, ?, ?, ?, ?)`,
				args: [e.action, e.fullName, e.version, e.detailsJson, now]
			});
		}
	}
	stmts.push({
		sql: `INSERT INTO audit_log (action, full_name, details_json, created_at)
		      VALUES ('transfer', ?, ?, ?)`,
		args: [`@${scope}`, JSON.stringify({ newOwner, force }), now]
	});
	await db.batch(stmts);
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
	const bucket = r2Bucket(env);
	if (bucket) {
		await db.execute({
			sql: `INSERT INTO chunks (hash, size_bytes, bytes, first_seen_at)
			      VALUES (?, ?, x'', ?)
			      ON CONFLICT(hash) DO NOTHING`,
			args: [hash, sizeBytes, Math.floor(Date.now() / 1000)]
		});
		await r2PutIfMissing(bucket, hash, bytes);
		return;
	}
	await db.execute({
		sql: `INSERT INTO chunks (hash, size_bytes, bytes, first_seen_at)
		      VALUES (?, ?, ?, ?)
		      ON CONFLICT(hash) DO NOTHING`,
		args: [hash, sizeBytes, bytes, Math.floor(Date.now() / 1000)]
	});
}

let quotaState = freshQuotaState();

export function resetQuotaState(): void {
	quotaState = freshQuotaState();
}

export async function getChunkBytes(
	env: Record<string, string | undefined>,
	hash: string
): Promise<Uint8Array | null> {
	if (!/^[0-9a-f]{64}$/.test(hash)) return null;
	const db = getClient(env);
	if (!db) return null;
	await ensureSchema(db);
	const rs = await db.execute({
		sql: 'SELECT bytes FROM chunks WHERE hash = ?',
		args: [hash]
	});
	const row = rs.rows[0] as Record<string, unknown> | undefined;
	if (!row) return null;
	const raw = row['bytes'] as Uint8Array | ArrayBuffer;
	const inline = raw instanceof Uint8Array ? raw : new Uint8Array(raw);
	if (inline.length > 0) return inline;
	const bucket = r2Bucket(env);
	if (!bucket) return null;
	const bytes = await r2Get(bucket, hash);
	if (!bytes) return null;
	if ((await sha256Hex(bytes)) !== hash) throw new Error(`chunk ${hash} failed integrity check`);
	return bytes;
}

export async function storedChunkBytes(
	env: Record<string, string | undefined>
): Promise<number> {
	const db = getClient(env);
	if (!db) return 0;
	await ensureSchema(db);
	const rs = await db.execute('SELECT COALESCE(SUM(size_bytes), 0) AS n FROM chunks');
	return Number(rs.rows[0]?.['n'] ?? 0);
}

export async function enforceStorageQuota(
	env: Record<string, string | undefined>,
	incomingBytes: number
): Promise<void> {
	if (!quotaGuardEnabled(env)) return;
	const budget = quotaBudgetBytes(env);
	if (shouldSample(quotaState, budget, Math.random())) {
		quotaState.sampled = await storedChunkBytes(env);
		quotaState.localBytes = 0;
	}
	const projected = effectiveUsage(quotaState, budget) + incomingBytes / budget;
	if (projected >= 1) {
		throw new Error('quota-exceeded: R2 storage budget reached');
	}
	quotaState.localBytes += incomingBytes;
}

export async function linkVersionChunks(
	env: Record<string, string | undefined>,
	versionRowId: string,
	hashes: string[]
): Promise<void> {
	const db = getClient(env);
	if (!db) throw new Error('missing database');
	await ensureSchema(db);
	if (hashes.length === 0) return;
	await db.batch(
		hashes.map((hash, ord) => ({
			sql: `INSERT INTO manifest_chunks (chunk_hash, version_id, ord)
			      VALUES (?, ?, ?)`,
			args: [hash, versionRowId, ord]
		}))
	);
}

export async function storeTarManifest(
	env: Record<string, string | undefined>,
	versionRowId: string,
	entries: TarEntryRef[]
): Promise<void> {
	const db = getClient(env);
	if (!db) throw new Error('missing database');
	await ensureSchema(db);
	await db.execute({
		sql: 'UPDATE package_versions SET tar_manifest_json = ? WHERE id = ?',
		args: [JSON.stringify(entries), versionRowId]
	});
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
	// Deletion here is final past the 30d grace window. If accidental
	// loss ever matters, the upgrade path is quarantine-then-delete:
	// move orphans to a quarantine prefix with a second grace window
	// before hard delete - not a backup system.
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

// Never throws: a purge failure must not fail the mutation that triggered it.
export interface PurgeResult {
	ok: boolean;
	verified: boolean;
}

export async function purgeUrls(
	env: Record<string, string | undefined>,
	urls: string[],
	options?: { fullName?: string; prefixes?: string[] }
): Promise<PurgeResult> {
	if (urls.length === 0 && !(options?.prefixes?.length)) return { ok: true, verified: false };
	const zone = env['CF_ZONE_ID'];
	const token = env['CF_PURGE_TOKEN'];
	if (zone && token) {
		const body: Record<string, string[]> = { files: urls };
		if (options?.prefixes?.length) body['prefixes'] = options.prefixes;
		for (let attempt = 0; attempt < 3; attempt++) {
			try {
				const res = await fetch(`https://api.cloudflare.com/client/v4/zones/${zone}/purge_cache`, {
					method: 'POST',
					headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
					body: JSON.stringify(body),
					signal: AbortSignal.timeout(10000)
				});
				const data = (await res.json().catch(() => null)) as { success?: boolean } | null;
				if (res.ok && data?.success === true) return { ok: true, verified: true };
			} catch {
				// retried on the next attempt; the origin stays authoritative
			}
		}
		try {
			const db = getClient(env);
			if (!db) return { ok: false, verified: false };
			await ensureSchema(db);
			await db.execute({
				sql: `INSERT INTO audit_log (action, full_name, details_json, created_at)
				      VALUES ('purge-failed', ?, ?, ?)`,
				args: [options?.fullName ?? '', JSON.stringify({ urls, prefixes: options?.prefixes ?? [] }), Math.floor(Date.now() / 1000)]
			});
		} catch {
			// audit write is best-effort too
		}
		return { ok: false, verified: false };
	}
	try {
		const db = getClient(env);
		if (!db) return { ok: false, verified: false };
		await ensureSchema(db);
		await db.execute({
			sql: `INSERT INTO audit_log (action, full_name, details_json, created_at)
			      VALUES ('purge-skipped', ?, ?, ?)`,
			args: [options?.fullName ?? '', JSON.stringify({ urls, prefixes: options?.prefixes ?? [] }), Math.floor(Date.now() / 1000)]
		});
		return { ok: true, verified: false };
	} catch {
		return { ok: false, verified: false };
	}
}

export function packagePointerUrls(origin: string, full: string, version?: string): string[] {
	const base = origin.replace(/\/$/, '');
	const urls = [`${base}/api/packages`, `${base}/api/packages/${full}`];
	if (version) {
		urls.push(`${base}/api/packages/${full}@${version}`);
		urls.push(`${base}/api/packages/${full}@${version}/api`);
		urls.push(`${base}/api/packages/${full}@${version}/manifest`);
		urls.push(`${base}/api/packages/${full}@${version}/chunks`);
		urls.push(`${base}/api/packages/${full}@${version}/guides`);
	}
	return urls;
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

export function pruneHit(have: HaveEntry[], full: string, version: string, sha: string): boolean {
	if (!sha) return false;
	return have.some((h) => h.full === full && h.version === version && h.integrity === sha);
}

export async function resolveGraph(
	env: Record<string, string | undefined>,
	requirements: Record<string, string>,
	have: HaveEntry[]
): Promise<{ resolved: Record<string, string>; levels: ResolveNode[][] }> {
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
			if (pruneHit(have, full, pick, row.tarballSha256)) {
				memo.set(id, {
					full,
					version: pick,
					path: `${full}@${pick}/chunks`,
					integrity: row.tarballSha256,
					engineRange: row.engineRange,
					deps: [],
					yanked: row.status === 'yanked'
				});
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
				path: `${full}@${pick}/chunks`,
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
	const cur = await db.execute({
		sql: 'SELECT manifest_json FROM package_versions WHERE package_id = ? AND version = ?',
		args: [id, version]
	});
	let manifestJson = String(cur.rows[0]?.['manifest_json'] ?? '{}');
	try {
		const manifest = JSON.parse(manifestJson) as Record<string, unknown>;
		manifest['withdrawReason'] = reason;
		manifestJson = JSON.stringify(manifest);
	} catch {
		manifestJson = JSON.stringify({ withdrawReason: reason });
	}
	await db.batch([
		{
			sql: `INSERT INTO tombstones (package_id, version, reason, created_at)
			      VALUES (?, ?, ?, ?)
			      ON CONFLICT(package_id, version) DO NOTHING`,
			args: [id, version, reason, now]
		},
		{
			sql: `UPDATE package_versions SET status = 'tombstoned', manifest_json = ?
			      WHERE package_id = ? AND version = ?`,
			args: [manifestJson, id, version]
		},
		{
			sql: `INSERT INTO audit_log (action, full_name, version, details_json, created_at)
			      VALUES ('takedown', ?, ?, ?, ?)`,
			args: [parsed.full, version, JSON.stringify({ reason }), now]
		}
	]);
}
