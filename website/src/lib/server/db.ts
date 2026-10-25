import { createClient, type Client } from '@libsql/client/web';
import { drizzle } from 'drizzle-orm/libsql/web';
import type { BatchItem } from 'drizzle-orm/batch';
import { alias } from 'drizzle-orm/sqlite-core';
import { and, asc, count, desc, eq, gte, inArray, lt, notExists, or, sql, sum } from 'drizzle-orm';
import * as s from './schema.js';
import { maxSatisfying, parseSemver, levelize, satisfiesRange, selectLatestVersion } from './registry.js';
import { sha256Hex, type TarEntryRef } from './chunks.js';
import { r2Bucket, r2Put, r2Get } from './r2.js';
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

export interface VersionCapability {
	domain: string;
	arg: string;
	reason: string | null;
}

export interface VersionDoc {
	version: string;
	readme: string;
	docJson: string;
	guides: string;
	permissions: VersionCapability[];
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
    license_id INTEGER REFERENCES licenses(id),
    downloads INTEGER DEFAULT 0,
    stars INTEGER DEFAULT 0,
    tags TEXT NOT NULL,
    keywords TEXT DEFAULT '',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    mutation_count INTEGER NOT NULL DEFAULT 0,
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
    manifest_digest TEXT NOT NULL DEFAULT '',
    verdict_key TEXT NOT NULL DEFAULT '',
    artifact_digest TEXT NOT NULL DEFAULT '',
    artifact_meta TEXT NOT NULL DEFAULT '',
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

CREATE TABLE IF NOT EXISTS advisories (
    id TEXT PRIMARY KEY,
    package TEXT NOT NULL,
    affected TEXT NOT NULL DEFAULT '*',
    artifacts TEXT NOT NULL DEFAULT '[]',
    severity TEXT NOT NULL DEFAULT 'warning',
    title TEXT NOT NULL DEFAULT '',
    detail TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'active',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_advisories_package ON advisories(package, state);

CREATE TABLE IF NOT EXISTS chunks (
    hash TEXT PRIMARY KEY,
    size_bytes INTEGER NOT NULL,
    bytes BLOB NOT NULL,
    first_seen_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS canonical_facts (
    canonical_hash TEXT PRIMARY KEY,
    chunk_list TEXT NOT NULL DEFAULT '[]',
    import_list TEXT NOT NULL DEFAULT '[]',
    evidence TEXT NOT NULL DEFAULT '[]',
    first_seen_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS package_edges (
    package_id TEXT PRIMARY KEY,
    edge_list TEXT NOT NULL DEFAULT '[]',
    perms_digest TEXT NOT NULL DEFAULT '',
    file_list TEXT NOT NULL DEFAULT '[]',
    updated_at INTEGER NOT NULL
);

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

CREATE TABLE IF NOT EXISTS capability_domains (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS version_capabilities (
    version_id TEXT NOT NULL REFERENCES package_versions(id) ON DELETE CASCADE,
    domain_id INTEGER NOT NULL REFERENCES capability_domains(id),
    arg TEXT NOT NULL DEFAULT '',
    reason TEXT,
    PRIMARY KEY (version_id, domain_id, arg)
);
CREATE INDEX IF NOT EXISTS idx_vc_domain ON version_capabilities(domain_id);

CREATE TABLE IF NOT EXISTS licenses (
    id INTEGER PRIMARY KEY,
    spdx TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
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

function connectDrizzle(raw: Client) {
	return drizzle({ client: raw, schema: s });
}

export type Ddb = ReturnType<typeof connectDrizzle>;

let ddb: Ddb | null = null;

export async function getDdb(env: Record<string, string | undefined>): Promise<Ddb | null> {
	const raw = getClient(env);
	if (!raw) return null;
	await ensureSchema(raw);
	if (!ddb) ddb = connectDrizzle(raw);
	return ddb;
}

export async function ensureSchema(db: Client): Promise<void> {
	if (schemaReady) return;
	if (schemaPromise) return schemaPromise;
	schemaPromise = checkSchemaVersion(db).then(
		(ok) => {
			if (ok) {
				schemaReady = true;
				return;
			}
			return migrateSchema(db).then(() => {
				schemaReady = true;
			});
		},
		(e) => {
			schemaPromise = null;
			throw e;
		}
	);
	return schemaPromise;
}

const SCHEMA_VERSION = 6;

async function checkSchemaVersion(db: Client): Promise<boolean> {
	try {
		const rs = await db.execute({
			sql: 'SELECT value FROM meta WHERE key = ?',
			args: ['schema_version']
		});
		return String(rs.rows[0]?.['value'] ?? '') === String(SCHEMA_VERSION);
	} catch {
		return false;
	}
}

async function stampSchemaVersion(db: Client): Promise<void> {
	await db.execute({
		sql: `INSERT INTO meta (key, value) VALUES ('schema_version', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value`,
		args: [String(SCHEMA_VERSION)]
	});
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
		['manifest_digest', "TEXT NOT NULL DEFAULT ''"],
		['verdict_key', "TEXT NOT NULL DEFAULT ''"],
		['artifact_digest', "TEXT NOT NULL DEFAULT ''"],
		['artifact_meta', "TEXT NOT NULL DEFAULT ''"],
		['request_id', "TEXT NOT NULL DEFAULT ''"]
	];
	for (const [col, ddl] of backfill) {
		if (!vnames.has(col)) {
			await db.execute(`ALTER TABLE package_versions ADD COLUMN ${col} ${ddl}`);
		}
	}
	await db.execute(`CREATE TABLE IF NOT EXISTS canonical_facts (
	    canonical_hash TEXT PRIMARY KEY,
	    chunk_list TEXT NOT NULL DEFAULT '[]',
	    import_list TEXT NOT NULL DEFAULT '[]',
	    evidence TEXT NOT NULL DEFAULT '[]',
	    first_seen_at INTEGER NOT NULL
	)`);
	await db.execute(`CREATE TABLE IF NOT EXISTS package_edges (
	    package_id TEXT PRIMARY KEY,
	    edge_list TEXT NOT NULL DEFAULT '[]',
	    perms_digest TEXT NOT NULL DEFAULT '',
	    file_list TEXT NOT NULL DEFAULT '[]',
	    updated_at INTEGER NOT NULL
	)`);
	// The per-version chunk mapping table is gone: liveness derives from
	// the version rows' manifests, so the mapping is never written.
	await db.execute('DROP TABLE IF EXISTS manifest_chunks');
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
	if (!names.has('license_id')) {
		await db.execute('ALTER TABLE packages ADD COLUMN license_id INTEGER REFERENCES licenses(id)');
	}
	if (!names.has('mutation_count')) {
		await db.execute('ALTER TABLE packages ADD COLUMN mutation_count INTEGER NOT NULL DEFAULT 0');
	}
	// The global generation counter is gone: the per-package mutation count
	// supersedes it and nothing reads it. The row is dropped once, here.
	await db.execute("DELETE FROM meta WHERE key = 'registry_generation'");
	await db.execute("UPDATE packages SET owner = 'org' WHERE owner = ''");
	// License normalization: keep the TEXT column during transition, but
	// point every row at its licenses FK entry.
	await db.execute(
		'INSERT INTO licenses (spdx) SELECT DISTINCT license FROM packages WHERE license IS NOT NULL ON CONFLICT(spdx) DO NOTHING'
	);
	await db.execute(
		'UPDATE packages SET license_id = (SELECT id FROM licenses WHERE spdx = packages.license) WHERE license_id IS NULL'
	);
	// The mapping table is dropped above; an old-shape survivor is
	// dropped here rather than migrated, since nothing reads it.
	await stampSchemaVersion(db);
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

interface SummarySource {
	scope: string;
	name: string;
	description: string;
	author: string;
	repository: string | null;
	license: string;
	downloads: number;
	stars: number;
	tags: string;
	keywords: string | null;
	updatedAt: number;
}

function toSummary(
	r: SummarySource,
	latest: string,
	versionCount: number,
	dependents: number = 0
): PackageSummary {
	return {
		name: r.scope ? `@${r.scope}/${r.name}` : r.name,
		description: r.description,
		author: r.author,
		repository: r.repository ?? '',
		license: r.license ?? 'MIT',
		downloads: r.downloads ?? 0,
		stars: r.stars ?? 0,
		tags: (r.tags ?? '')
			.split(',')
			.map((t) => t.trim())
			.filter(Boolean),
		keywords: (r.keywords ?? '')
			.split(',')
			.map((t) => t.trim())
			.filter(Boolean),
		updatedAt: r.updatedAt ?? 0,
		latest,
		versionCount,
		dependents
	};
}

export async function listPackages(
	env: Record<string, string | undefined>,
	filters: PackageFilter = {}
): Promise<PackageSummary[]> {
	const db = await getDdb(env);
	if (!db) return [];
	const now = Math.floor(Date.now() / 1000);
	const rows = await db.select().from(s.packages).orderBy(desc(s.packages.updatedAt));
	const vers = await db
		.select({
			packageId: s.packageVersions.packageId,
			version: s.packageVersions.version,
			status: s.packageVersions.status
		})
		.from(s.packageVersions);
	const byPkg = new Map<string, Array<{ version: string; status: string }>>();
	for (const v of vers) {
		const list = byPkg.get(v.packageId) ?? [];
		list.push({ version: v.version, status: v.status ?? 'live' });
		byPkg.set(v.packageId, list);
	}
	let sums = new Map<string, number>();
	try {
		const ds = await db
			.select({ packageId: s.downloadDaily.packageId, n: sum(s.downloadDaily.downloads) })
			.from(s.downloadDaily)
			.where(gte(s.downloadDaily.day, dayString(now - 30 * 86400)))
			.groupBy(s.downloadDaily.packageId);
		for (const r of ds) sums.set(r.packageId, Number(r.n ?? 0));
	} catch {
		sums = new Map();
	}
	let depCounts = new Map<string, number>();
	try {
		const dc = await db
			.select({ depName: s.packageDeps.depName, n: count() })
			.from(s.packageDeps)
			.groupBy(s.packageDeps.depName);
		for (const r of dc) depCounts.set(r.depName, Number(r.n ?? 0));
	} catch {
		depCounts = new Map();
	}
	const out: PackageSummary[] = [];
	for (const r of rows) {
		const versions = byPkg.get(r.id) ?? [];
		const summary = toSummary(
			{
				scope: r.scope,
				name: r.name,
				description: r.description,
				author: r.author,
				repository: r.repository,
				license: r.license,
				downloads: sums.get(r.id) ?? 0,
				stars: r.stars,
				tags: r.tags,
				keywords: r.keywords,
				updatedAt: r.updatedAt
			},
			selectLatestVersion(versions) ?? '',
			versions.length,
			0
		);
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
	const db = await getDdb(env);
	if (!db) return null;
	const row = await db
		.select({ owner: s.packages.owner })
		.from(s.packages)
		.where(and(eq(s.packages.scope, scope), eq(s.packages.name, name)))
		.get();
	return row ? row.owner : null;
}

export async function getDependents(
	env: Record<string, string | undefined>,
	full: string
): Promise<string[]> {
	const parsed = parsePackageName(full);
	const db = await getDdb(env);
	if (!parsed || !db) return [];
	const rows = await db
		.select({ scope: s.packages.scope, name: s.packages.name })
		.from(s.packageDeps)
		.innerJoin(s.packages, eq(s.packages.id, s.packageDeps.packageId))
		.where(eq(s.packageDeps.depName, parsed.full))
		.orderBy(asc(s.packages.scope), asc(s.packages.name));
	return rows.map((r) => (r.scope ? `@${r.scope}/${r.name}` : r.name));
}

export async function getPackage(
	env: Record<string, string | undefined>,
	full: string
): Promise<PackageDetail | null> {
	const parsed = parsePackageName(full);
	const db = await getDdb(env);
	if (!parsed || !db) return null;
	const row = await db
		.select()
		.from(s.packages)
		.where(and(eq(s.packages.scope, parsed.scope), eq(s.packages.name, parsed.name)))
		.get();
	if (!row) return null;
	const vrows = await db
		.select({
			version: s.packageVersions.version,
			checksum: s.packageVersions.checksum,
			status: s.packageVersions.status,
			createdAt: s.packageVersions.createdAt
		})
		.from(s.packageVersions)
		.where(eq(s.packageVersions.packageId, row.id))
		.orderBy(desc(s.packageVersions.createdAt));
	const versions: VersionMeta[] = vrows.map((v) => ({
		version: v.version,
		checksum: v.checksum,
		status: v.status ?? 'live',
		createdAt: v.createdAt
	}));
	if (versions.length === 0) return null;
	const depName = parsed.scope ? `@${parsed.scope}/${parsed.name}` : parsed.name;
	const depRow = await db
		.select({ n: count() })
		.from(s.packageDeps)
		.where(eq(s.packageDeps.depName, depName))
		.get();
	return {
		...toSummary(
			{
				scope: row.scope,
				name: row.name,
				description: row.description,
				author: row.author,
				repository: row.repository,
				license: row.license,
				downloads: row.downloads,
				stars: row.stars,
				tags: row.tags,
				keywords: row.keywords,
				updatedAt: row.updatedAt
			},
			selectLatestVersion(versions) ?? versions[0].version,
			versions.length,
			Number(depRow?.n ?? 0)
		),
		dependencies: [],
		createdAt: row.createdAt ?? 0,
		versions
	};
}

export async function getVersionDoc(
	env: Record<string, string | undefined>,
	full: string,
	version: string
): Promise<VersionDoc | null> {
	const parsed = parsePackageName(full);
	const db = await getDdb(env);
	if (!parsed || !db) return null;
	// One query: the version payload plus its declared permissions, if any.
	const rows = await db.all<{
		readme: string;
		docJson: string;
		guides: string;
		domain: string | null;
		arg: string | null;
		reason: string | null;
	}>(sql`SELECT v.readme_markdown AS readme, v.doc_json AS docJson, v.guides_json AS guides,
		d.name AS domain, c.arg AS arg, c.reason AS reason
		FROM package_versions v
		JOIN packages p ON p.id = v.package_id
		LEFT JOIN version_capabilities c ON c.version_id = v.id
		LEFT JOIN capability_domains d ON d.id = c.domain_id
		WHERE p.scope = ${parsed.scope} AND p.name = ${parsed.name} AND v.version = ${version}
		ORDER BY d.name, c.arg`);
	if (rows.length === 0) return null;
	const first = rows[0];
	const permissions: VersionCapability[] = [];
	for (const r of rows) {
		if (r.domain !== null) {
			permissions.push({ domain: r.domain, arg: r.arg ?? '', reason: r.reason ?? null });
		}
	}
	return {
		version,
		readme: first.readme,
		docJson: first.docJson,
		guides: first.guides ?? '[]',
		permissions
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
	capabilities?: Array<{ domain: string; arg: string; reason: string | null }>;
}

export interface PublishResult {
	name: string;
	version: string;
	created: boolean;
}

export interface CanonicalUnit {
	hash: string;
	size: number;
	bytes: Uint8Array;
}

export interface CanonicalFact {
	canonicalHash: string;
	chunks: string[];
	imports: string[];
	evidence: string[];
}

export interface CanonicalEdges {
	edges: string[];
	permsDigest: string;
	files: Array<{ path: string; hash: string }>;
}

export interface CanonicalPublishPayload extends PublishPayload {
	entries: TarEntryRef[];
	manifestDigest: string;
	verdictKey: string;
	artifactDigest: string | null;
	artifactMeta: string | null;
	units: CanonicalUnit[];
	facts: CanonicalFact[];
	edges: CanonicalEdges;
}

// Abuse guards (free-tier survival): hard caps enforced before any write.
// - Payload caps keep single publishes small.
// - MAX_VERSIONS_PER_WEEK bounds churn per package on a rolling window.
export const MAX_DOC_JSON_BYTES = 1024 * 1024;
export const MAX_README_BYTES = 200 * 1024;
export const MAX_VERSIONS_PER_WEEK = 20;

export async function publishCanonicalVersion(
	env: Record<string, string | undefined>,
	p: CanonicalPublishPayload
): Promise<PublishResult> {
	const parsed = parsePackageName(p.name);
	const db = await getDdb(env);
	if (!parsed || !db) throw new Error('invalid package name or missing database');
	const sem = parseSemver(p.version);
	if (!sem) throw new Error('invalid package version (expected X.Y.Z)');
	const now = Math.floor(Date.now() / 1000);
	const id = packageId(parsed.scope, parsed.name);
	const auditJson = JSON.stringify({ manifestDigest: p.manifestDigest });
	const keywords = (p.keywords ?? []).join(',');
	// Dependents reflect ever-depending packages (npm shows all): rows are
	// derived from manifests at publish time and are never deleted on
	// yank or tombstone, so no counter can desync under retries.
	const depNames = extractDepNames(p.manifestJson ?? '{}');
	const capabilities = p.capabilities ?? [];
	const versionId = `ver_${id}_${p.version}`;
	const versionInsert = db
		.insert(s.packageVersions)
		.values({
			id: versionId,
			packageId: id,
			version: p.version,
			readmeMarkdown: p.readme,
			docJson: p.docJson,
			checksum: p.manifestDigest,
			status: 'live',
			createdAt: now,
			semverMajor: sem.major,
			semverMinor: sem.minor,
			semverPatch: sem.patch,
			prerelease: sem.prerelease,
			engineRange: p.engineRange ?? '',
			manifestJson: p.manifestJson ?? '{}',
			guidesJson: p.guidesJson ?? '[]',
			tarballSha256: '',
			tarManifestJson: JSON.stringify(p.entries),
			manifestDigest: p.manifestDigest,
			verdictKey: p.verdictKey,
			artifactDigest: p.artifactDigest ?? '',
			artifactMeta: p.artifactMeta ?? '',
			requestId: p.requestId ?? ''
		})
		.onConflictDoNothing();
	// The audit row is the statement right after the version insert: it reads
	// changes() from that insert, so it lands only when this publish minted
	// the version. Anything between the two would change what changes()
	// reports and turn every republish into a second audit row.
	const publishAudit = db.run(
		sql`INSERT INTO audit_log (action, full_name, version, details_json, request_id, created_at)
		      SELECT 'publish', ${parsed.full}, ${p.version}, ${auditJson}, ${p.requestId ?? ''}, ${now} WHERE changes() > 0`
	);
	const domainNames = [...new Set(capabilities.map((c) => c.domain))].sort();
	const bucket = r2Bucket(env);
	const batch: unknown[] = [
		db
			.insert(s.scopes)
			.values({
				name: parsed.scope,
				owner: ORG_OWNER,
				reserved: RESERVED_SCOPES.includes(parsed.scope) ? 1 : 0,
				createdAt: now
			})
			.onConflictDoNothing(),
		...(p.license
			? [
					db
						.insert(s.licenses)
						.values({ spdx: p.license })
						.onConflictDoNothing()
				]
			: []),
		...domainNames.map((name) =>
			db.insert(s.capabilityDomains).values({ name }).onConflictDoNothing()
		),
		db
			.insert(s.packages)
			.values({
				id,
				scope: parsed.scope,
				name: parsed.name,
				owner: p.owner ?? '',
				description: p.description,
				author: p.author,
				repository: '',
				license: p.license,
				licenseId: p.license
					? sql`(SELECT id FROM licenses WHERE spdx = ${p.license})`
					: null,
				downloads: 0,
				stars: 0,
				tags: p.tags.join(','),
				keywords,
				createdAt: now,
				updatedAt: now,
				mutationCount: 1
			})
			.onConflictDoUpdate({
				target: [s.packages.scope, s.packages.name],
				set: {
					description: sql`excluded.description`,
					keywords: sql`excluded.keywords`,
					updatedAt: sql`excluded.updated_at`,
					// The publish already writes this row, so the candidate-set
					// counter rides along: no extra row is written.
					mutationCount: sql`${s.packages.mutationCount} + 1`
				}
			}),
		versionInsert,
		publishAudit,
		...depNames.map((dep) =>
			db
				.insert(s.packageDeps)
				.values({ packageId: id, depName: dep })
				.onConflictDoNothing()
		),
		...capabilities.map((c) =>
			db
				.insert(s.versionCapabilities)
				.values({
					versionId,
					domainId: sql`(SELECT id FROM capability_domains WHERE name = ${c.domain})`,
					arg: c.arg,
					reason: c.reason
				})
				.onConflictDoNothing()
		),
		// Chunk rows land in the same batch as the version row, and only
		// after their R2 PUTs succeeded (the route PUTs first). A chunk row
		// therefore means the bytes exist: the chunks table is the
		// existence oracle, and no R2 probe is ever needed.
		...p.units.map((unit) =>
			bucket
				? db.run(sql`INSERT INTO chunks (hash, size_bytes, bytes, first_seen_at)
				             VALUES (${unit.hash}, ${unit.size}, x'', ${now})
				             ON CONFLICT(hash) DO NOTHING`)
				: db.run(sql`INSERT INTO chunks (hash, size_bytes, bytes, first_seen_at)
				             VALUES (${unit.hash}, ${unit.size}, ${unit.bytes}, ${now})
				             ON CONFLICT(hash) DO NOTHING`)
		),
		...p.facts.map((fact) =>
			db
				.insert(s.canonicalFacts)
				.values({
					canonicalHash: fact.canonicalHash,
					chunkList: JSON.stringify(fact.chunks),
					importList: JSON.stringify(fact.imports),
					evidence: JSON.stringify(fact.evidence),
					firstSeenAt: now
				})
				.onConflictDoNothing()
		),
		db
			.insert(s.packageEdges)
			.values({
				packageId: id,
				edgeList: JSON.stringify(p.edges.edges),
				permsDigest: p.edges.permsDigest,
				fileList: JSON.stringify(p.edges.files),
				updatedAt: now
			})
			.onConflictDoUpdate({
				target: s.packageEdges.packageId,
				set: {
					edgeList: sql`excluded.edge_list`,
					permsDigest: sql`excluded.perms_digest`,
					fileList: sql`excluded.file_list`,
					updatedAt: sql`excluded.updated_at`
				}
			})
	];
	// The batch order is the publish idempotency contract, so it is checked
	// instead of being remembered: the audit statement reads the version
	// insert, so nothing may land between the two.
	const versionIndex = batch.indexOf(versionInsert);
	const auditIndex = batch.indexOf(publishAudit);
	if (versionIndex < 0 || auditIndex !== versionIndex + 1) {
		throw new Error('publish batch: the audit row must follow the version insert');
	}
	const applied = await db.batch(batch as unknown as [BatchItem<'sqlite'>, ...BatchItem<'sqlite'>[]]);
	const created = (applied[versionIndex]?.rowsAffected ?? 0) > 0;
	noteMutation();
	return { name: parsed.full, version: p.version, created };
}

export async function recentVersionCount(
	env: Record<string, string | undefined>,
	full: string,
	sinceSeconds: number
): Promise<number> {
	const parsed = parsePackageName(full);
	const db = await getDdb(env);
	if (!parsed || !db) return 0;
	const row = await db
		.select({ n: count() })
		.from(s.packageVersions)
		.innerJoin(s.packages, eq(s.packages.id, s.packageVersions.packageId))
		.where(
			and(
				eq(s.packages.scope, parsed.scope),
				eq(s.packages.name, parsed.name),
				gte(s.packageVersions.createdAt, Math.floor(Date.now() / 1000) - sinceSeconds)
			)
		)
		.get();
	return Number(row?.n ?? 0);
}

export interface VersionRow {
	id: string;
	version: string;
	status: string;
	checksum: string;
	tarballSha256: string;
	manifestDigest: string;
	verdictKey: string;
	artifactDigest: string;
	artifactMeta: string;
	engineRange: string;
	docJson: string;
	manifestJson: string;
	guidesJson: string;
	tarManifestJson: string;
	requestId: string;
	createdAt: number;
}

// The version row without the doc payload. The doc payload is up to 1 MiB
// per row, and the file index, manifest and guide routes never read it, so
// they select the narrow projection instead.
export type VersionIndexRow = Omit<VersionRow, 'docJson'>;

export const SEMVER_ORDER = `semver_major DESC, semver_minor DESC, semver_patch DESC,
	CASE WHEN prerelease = '' THEN 1 ELSE 0 END DESC, prerelease DESC, v.created_at DESC`;

function versionIndexColumns(v: typeof s.packageVersions) {
	return {
		id: v.id,
		version: v.version,
		status: v.status,
		checksum: v.checksum,
		tarballSha256: v.tarballSha256,
		manifestDigest: v.manifestDigest,
		verdictKey: v.verdictKey,
		artifactDigest: v.artifactDigest,
		artifactMeta: v.artifactMeta,
		tarManifestJson: v.tarManifestJson,
		engineRange: v.engineRange,
		manifestJson: v.manifestJson,
		guidesJson: v.guidesJson,
		requestId: v.requestId,
		createdAt: v.createdAt
	};
}

export async function getVersionRow(
	env: Record<string, string | undefined>,
	full: string,
	version: string
): Promise<{ row: VersionRow; withdrawn: boolean } | null> {
	const parsed = parsePackageName(full);
	const db = await getDdb(env);
	if (!parsed || !db) return null;
	const v = s.packageVersions;
	const p = s.packages;
	const r = await db
		.select({
			...versionIndexColumns(v),
			docJson: v.docJson
		})
		.from(v)
		.innerJoin(p, eq(p.id, v.packageId))
		.where(and(eq(p.scope, parsed.scope), eq(p.name, parsed.name), eq(v.version, version)))
		.get();
	if (!r) return null;
	const row: VersionRow = {
		id: r.id,
		version: r.version,
		status: r.status ?? 'live',
		checksum: r.checksum ?? '',
		tarballSha256: r.tarballSha256 ?? '',
		manifestDigest: r.manifestDigest ?? '',
		verdictKey: r.verdictKey ?? '',
		artifactDigest: r.artifactDigest ?? '',
		artifactMeta: r.artifactMeta ?? '',
		tarManifestJson: r.tarManifestJson ?? '[]',
		engineRange: r.engineRange ?? '',
		docJson: r.docJson ?? '{"modules":[]}',
		manifestJson: r.manifestJson ?? '{}',
		guidesJson: r.guidesJson ?? '[]',
		requestId: r.requestId ?? '',
		createdAt: r.createdAt ?? 0
	};
	const withdrawn = row.status === 'tombstoned' || (await isVersionWithdrawn(env, full, version));
	return { row, withdrawn };
}

// The same row without the doc payload, for the routes that never read it.
export async function getVersionIndexRow(
	env: Record<string, string | undefined>,
	full: string,
	version: string
): Promise<{ row: VersionIndexRow; withdrawn: boolean } | null> {
	const parsed = parsePackageName(full);
	const db = await getDdb(env);
	if (!parsed || !db) return null;
	const v = s.packageVersions;
	const p = s.packages;
	const r = await db
		.select(versionIndexColumns(v))
		.from(v)
		.innerJoin(p, eq(p.id, v.packageId))
		.where(and(eq(p.scope, parsed.scope), eq(p.name, parsed.name), eq(v.version, version)))
		.get();
	if (!r) return null;
	const row: VersionIndexRow = {
		id: r.id,
		version: r.version,
		status: r.status ?? 'live',
		checksum: r.checksum ?? '',
		tarballSha256: r.tarballSha256 ?? '',
		manifestDigest: r.manifestDigest ?? '',
		verdictKey: r.verdictKey ?? '',
		artifactDigest: r.artifactDigest ?? '',
		artifactMeta: r.artifactMeta ?? '',
		tarManifestJson: r.tarManifestJson ?? '[]',
		engineRange: r.engineRange ?? '',
		manifestJson: r.manifestJson ?? '{}',
		guidesJson: r.guidesJson ?? '[]',
		requestId: r.requestId ?? '',
		createdAt: r.createdAt ?? 0
	};
	const withdrawn = row.status === 'tombstoned' || (await isVersionWithdrawn(env, full, version));
	return { row, withdrawn };
}

export async function getPackageEdges(
	env: Record<string, string | undefined>,
	full: string
): Promise<{ edges: string[]; permsDigest: string; files: Array<{ path: string; hash: string }> } | null> {
	const parsed = parsePackageName(full);
	const db = await getDdb(env);
	if (!parsed || !db) return null;
	const row = await db
		.select({
			edgeList: s.packageEdges.edgeList,
			permsDigest: s.packageEdges.permsDigest,
			fileList: s.packageEdges.fileList
		})
		.from(s.packageEdges)
		.where(eq(s.packageEdges.packageId, packageId(parsed.scope, parsed.name)))
		.get();
	if (!row) return null;
	try {
		const edges = JSON.parse(String(row.edgeList ?? '[]')) as unknown;
		const files = JSON.parse(String(row.fileList ?? '[]')) as unknown;
		return {
			edges: Array.isArray(edges) ? edges.filter((e): e is string => typeof e === 'string') : [],
			permsDigest: String(row.permsDigest ?? ''),
			files: Array.isArray(files) ? (files as Array<{ path: string; hash: string }>) : []
		};
	} catch {
		return null;
	}
}

export async function listLiveVersionsBatch(
	env: Record<string, string | undefined>,
	fulls: string[]
): Promise<Map<string, string[]>> {
	const out = new Map<string, string[]>();
	const want = fulls
		.map((full) => ({ full, parsed: parsePackageName(full) }))
		.filter((w): w is { full: string; parsed: NonNullable<ReturnType<typeof parsePackageName>> } => w.parsed !== null);
	if (want.length === 0) return out;
	const db = await getDdb(env);
	if (!db) return out;
	const v = alias(s.packageVersions, 'v');
	const p = s.packages;
	const rows = await db
		.select({ scope: p.scope, name: p.name, version: v.version })
		.from(v)
		.innerJoin(p, eq(p.id, v.packageId))
		.where(
			and(
				or(...want.map((w) => and(eq(p.scope, w.parsed.scope), eq(p.name, w.parsed.name)))),
				inArray(v.status, ['live', 'yanked'])
			)
		)
		.orderBy(p.scope, p.name, sql.raw(SEMVER_ORDER));
	for (const r of rows) {
		const full = r.scope ? `@${r.scope}/${r.name}` : r.name;
		const list = out.get(full) ?? [];
		list.push(r.version);
		out.set(full, list);
	}
	return out;
}

export async function listLiveVersions(
	env: Record<string, string | undefined>,
	full: string
): Promise<string[]> {
	const parsed = parsePackageName(full);
	const db = await getDdb(env);
	if (!parsed || !db) return [];
	// Aliased as v: SEMVER_ORDER qualifies created_at as v.created_at,
	// and the unqualified semver columns resolve to this table.
	const v = alias(s.packageVersions, 'v');
	const p = s.packages;
	const rows = await db
		.select({ version: v.version })
		.from(v)
		.innerJoin(p, eq(p.id, v.packageId))
		.where(
			and(
				eq(p.scope, parsed.scope),
				eq(p.name, parsed.name),
				inArray(v.status, ['live', 'yanked'])
			)
		)
		.orderBy(sql.raw(SEMVER_ORDER));
	return rows.map((r) => r.version);
}

export async function getLatestVersion(
	env: Record<string, string | undefined>,
	full: string
): Promise<string | null> {
	const parsed = parsePackageName(full);
	const db = await getDdb(env);
	if (!parsed || !db) return null;
	const v = s.packageVersions;
	const p = s.packages;
	const rows = await db
		.select({ version: v.version, status: v.status })
		.from(v)
		.innerJoin(p, eq(p.id, v.packageId))
		.where(and(eq(p.scope, parsed.scope), eq(p.name, parsed.name)));
	return selectLatestVersion(rows.map((r) => ({ version: r.version, status: r.status ?? 'live' })));
}

export async function yankVersion(
	env: Record<string, string | undefined>,
	full: string,
	version: string
): Promise<boolean> {
	const parsed = parsePackageName(full);
	const db = await getDdb(env);
	if (!parsed || !db) throw new Error('invalid package name or missing database');
	const id = packageId(parsed.scope, parsed.name);
	const now = Math.floor(Date.now() / 1000);
	const applied = await db.batch([
		db
			.update(s.packageVersions)
			.set({ status: 'yanked' })
			.where(
				and(
					eq(s.packageVersions.packageId, id),
					eq(s.packageVersions.version, version),
					eq(s.packageVersions.status, 'live')
				)
			),
		db.run(
			sql`INSERT INTO audit_log (action, full_name, version, created_at)
			      SELECT 'yank', ${parsed.full}, ${version}, ${now} WHERE changes() > 0`
		)
	] as unknown as [BatchItem<'sqlite'>, ...BatchItem<'sqlite'>[]]);
	// A yank does not change the candidate set: a yanked version stays a
	// candidate, and the picked row is read fresh on every resolve. The
	// per-package mutation count therefore does not move.
	noteMutation();
	return (applied[0]?.rowsAffected ?? 0) > 0;
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
	const db = await getDdb(env);
	if (!db) throw new Error('missing database');
	const v = s.packageVersions;
	const p = s.packages;
	if (!force) {
		const live = await db
			.select({ n: count() })
			.from(v)
			.innerJoin(p, eq(p.id, v.packageId))
			.where(and(eq(p.scope, scope), inArray(v.status, ['live', 'yanked'])))
			.get();
		if (Number(live?.n ?? 0) > 0) {
			throw new Error('scope has live versions (retry with force)');
		}
	}
	const now = Math.floor(Date.now() / 1000);
	const pkgs = await db
		.select({ id: p.id, name: p.name })
		.from(p)
		.where(eq(p.scope, scope));
	const renames = pkgs.map((r) => ({
		oldId: r.id,
		name: r.name,
		newId: packageId(newOwner, r.name)
	}));
	const vrows =
		renames.length > 0
			? await db
					.select({ packageId: v.packageId, version: v.version })
					.from(v)
					.where(inArray(v.packageId, renames.map((r) => r.oldId)))
			: [];
	const byPkg = new Map<string, string[]>();
	for (const row of vrows) {
		const list = byPkg.get(row.packageId) ?? [];
		list.push(row.version);
		byPkg.set(row.packageId, list);
	}
	const stmts: Array<BatchItem<'sqlite'>> = [];
	// Child rows migrate before the parent PK so immediate FK checks never
	// see a dangling reference mid-batch.
	for (const r of renames) {
		stmts.push(
			db.update(v).set({ packageId: r.newId }).where(eq(v.packageId, r.oldId))
		);
		stmts.push(
			db.update(s.tombstones).set({ packageId: r.newId }).where(eq(s.tombstones.packageId, r.oldId))
		);
		stmts.push(
			db.update(s.packageDeps).set({ packageId: r.newId }).where(eq(s.packageDeps.packageId, r.oldId))
		);
		stmts.push(
			db.update(s.downloadDaily).set({ packageId: r.newId }).where(eq(s.downloadDaily.packageId, r.oldId))
		);
	}
	for (const r of renames) {
		stmts.push(
			db
				.update(p)
				.set({
					id: r.newId,
					scope: newOwner,
					updatedAt: now,
					// The scope move takes the versions out of the old name's
					// candidate set, so the counter moves with the row.
					mutationCount: sql`${p.mutationCount} + 1`
				})
				.where(eq(p.id, r.oldId))
		);
	}
	stmts.push(
		db
			.insert(s.scopes)
			.values({ name: newOwner, owner: newOwner, reserved: 0, createdAt: now })
			.onConflictDoNothing()
	);
	stmts.push(db.delete(s.scopes).where(eq(s.scopes.name, scope)));
	stmts.push(db.delete(s.orgs).where(eq(s.orgs.scope, scope)));
	for (const r of renames) {
		const oldFull = scope ? `@${scope}/${r.name}` : r.name;
		const newFull = `@${newOwner}/${r.name}`;
		for (const e of buildTransferAuditEntries(oldFull, newFull, byPkg.get(r.oldId) ?? [])) {
			stmts.push(
				db.insert(s.auditLog).values({
					action: e.action,
					fullName: e.fullName,
					version: e.version,
					detailsJson: e.detailsJson,
					createdAt: now
				})
			);
		}
	}
	stmts.push(
		db.insert(s.auditLog).values({
			action: 'transfer',
			fullName: `@${scope}`,
			version: '',
			detailsJson: JSON.stringify({ newOwner, force }),
			createdAt: now
		})
	);
	await db.batch(stmts as [BatchItem<'sqlite'>, ...BatchItem<'sqlite'>[]]);
	noteMutation();
}

// The skip-known check: chunk hashes the store already holds need no
// R2 PUT. The chunks table is the existence oracle (a row is written only
// after a successful PUT, in the same atomic batch as the version row),
// so no R2 existence probe is ever needed.
export async function existingChunkHashes(
	env: Record<string, string | undefined>,
	hashes: string[]
): Promise<Set<string>> {
	const db = await getDdb(env);
	const out = new Set<string>();
	if (!db || hashes.length === 0) return out;
	const uniq = [...new Set(hashes)];
	for (const batch of chunked(uniq, 900)) {
		const rows = await db
			.select({ hash: s.chunks.hash })
			.from(s.chunks)
			.where(inArray(s.chunks.hash, batch));
		for (const r of rows) out.add(r.hash);
	}
	return out;
}

export async function r2PutChunks(
	env: Record<string, string | undefined>,
	units: Array<{ hash: string; size: number; bytes: Uint8Array }>
): Promise<void> {
	const bucket = r2Bucket(env);
	if (!bucket) return;
	for (const unit of units) {
		// The object is keyed by its own digest, so the put is idempotent
		// and the existence probe the old path did is not needed.
		await r2Put(bucket, unit.hash, unit.bytes);
	}
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
	const db = await getDdb(env);
	if (!db) return null;
	const row = await db.get<{ bytes: unknown }>(sql`SELECT bytes FROM chunks WHERE hash = ${hash}`);
	if (!row) return null;
	const raw = (row as { bytes: Uint8Array | ArrayBuffer }).bytes;
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
	const db = await getDdb(env);
	if (!db) return 0;
	const row = await db.select({ n: sum(s.chunks.sizeBytes) }).from(s.chunks).get();
	return Number(row?.n ?? 0);
}

// The sources of a published package for analysis: reassembled from the
// version manifest through the chunks table, keyed by module rest
// (`bytes`, `net/http`) for std_provide. Only @std/* packages resolve
// this way; anything else is the publisher's own tree.
const MAX_DEP_SOURCES_BYTES = 4 * 1024 * 1024;

export async function getPublishedPackageSources(
	env: Record<string, string | undefined>,
	full: string
): Promise<{ version: string; files: Record<string, string> } | null> {
	const parsed = parsePackageName(full);
	if (!parsed || !/^[A-Za-z][A-Za-z0-9_]*$/.test(parsed.name)) return null;
	const top = parsed.name;
	const latest = await getLatestVersion(env, full);
	if (!latest) return null;
	const found = await getVersionRow(env, full, latest);
	if (!found || found.withdrawn) return null;
	let entries: TarEntryRef[];
	try {
		entries = JSON.parse(found.row.tarManifestJson) as TarEntryRef[];
		if (!Array.isArray(entries)) return null;
	} catch {
		return null;
	}
	const files: Record<string, string> = {};
	let total = 0;
	for (const e of entries) {
		if (!e || e.dir || !e.name.endsWith('.rnx')) continue;
		const stem = e.name.split('/').pop()?.replace(/\.rnx$/, '') ?? '';
		if (!/^[A-Za-z][A-Za-z0-9_]*$/.test(stem)) continue;
		const rest = stem === 'lib' ? top : `${top}/${stem}`;
		const parts: number[] = [];
		for (const h of e.chunks ?? []) {
			const bytes = await getChunkBytes(env, h);
			if (!bytes) return null;
			for (const b of bytes) parts.push(b);
		}
		const text = new TextDecoder().decode(new Uint8Array(parts));
		total += text.length;
		if (total > MAX_DEP_SOURCES_BYTES) throw new Error(`dependency @std/${top} sources exceed the analysis budget`);
		files[rest] = text;
	}
	if (Object.keys(files).length === 0) return null;
	return { version: latest, files };
}

// The storage quota of one upload. The check refuses an upload that does
// not fit the budget, and the charge counts the bytes of an upload that
// was accepted. They are two steps because the check runs before the
// expensive work of a publish: a refused upload costs the check and
// nothing else, while only a stored version counts against the budget.
export async function checkStorageQuota(
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
}

export function chargeStorageQuota(
	env: Record<string, string | undefined>,
	acceptedBytes: number
): void {
	if (!quotaGuardEnabled(env)) return;
	quotaState.localBytes += acceptedBytes;
}

// A chunk is live while a non-tombstoned, non-audit-only version row
// references it in its manifest. Liveness derives from the version rows
// directly: there is no mapping table to desync under retries.
export async function liveChunkHashes(env: Record<string, string | undefined>): Promise<Set<string>> {
	const db = await getDdb(env);
	const live = new Set<string>();
	if (!db) return live;
	let offset = 0;
	for (;;) {
		const rows = await db
			.select({ manifest: s.packageVersions.tarManifestJson })
			.from(s.packageVersions)
			.where(inArray(s.packageVersions.status, ['live', 'yanked']))
			.limit(500)
			.offset(offset);
		if (rows.length === 0) break;
		for (const row of rows) {
			try {
				const entries = JSON.parse(String(row.manifest ?? '[]')) as Array<{ chunks?: unknown }>;
				if (!Array.isArray(entries)) continue;
				for (const entry of entries) {
					if (!entry || !Array.isArray(entry.chunks)) continue;
					for (const hash of entry.chunks) {
						if (typeof hash === 'string' && /^[0-9a-f]{64}$/.test(hash)) live.add(hash);
					}
				}
			} catch {
				continue;
			}
		}
		offset += rows.length;
		if (rows.length < 500) break;
	}
	return live;
}

export async function findOrphanChunks(
	env: Record<string, string | undefined>,
	beforeUnix: number,
	limit: number
): Promise<string[]> {
	const db = await getDdb(env);
	if (!db) return [];
	const live = await liveChunkHashes(env);
	const out: string[] = [];
	let offset = 0;
	const want = Math.max(1, Math.min(1000, limit));
	for (;;) {
		const rows = await db
			.select({ hash: s.chunks.hash })
			.from(s.chunks)
			.where(lt(s.chunks.firstSeenAt, beforeUnix))
			.limit(500)
			.offset(offset);
		if (rows.length === 0) break;
		for (const r of rows) {
			if (!live.has(r.hash)) {
				out.push(r.hash);
				if (out.length >= want) return out;
			}
		}
		offset += rows.length;
		if (rows.length < 500) break;
	}
	return out;
}

export async function reverifyChunks(
	env: Record<string, string | undefined>,
	hashes: string[]
): Promise<Set<string>> {
	if (hashes.length === 0) return new Set();
	const live = await liveChunkHashes(env);
	const out = new Set<string>();
	for (const h of hashes) if (live.has(h)) out.add(h);
	return out;
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
	const db = await getDdb(env);
	if (!db || hashes.length === 0) return 0;
	let deleted = 0;
	for (const batch of chunked(hashes, 50)) {
		const res = await db.delete(s.chunks).where(inArray(s.chunks.hash, batch));
		deleted += res.rowsAffected ?? 0;
	}
	await db.insert(s.auditLog).values({
		action: 'gc-sweep',
		fullName: '',
		version: '',
		detailsJson: JSON.stringify({ deleted: hashes }),
		createdAt: Math.floor(Date.now() / 1000)
	});
	return deleted;
}

// Never throws: a purge failure must not fail the mutation that triggered it.
export interface PurgeResult {
	ok: boolean;
	verified: boolean;
}

export interface PurgeOptions {
	fullName?: string;
	prefixes?: string[];
	// The URL the edge is checked on, and what the edge has to answer with.
	// Without it the purge is accepted on the API answer alone, which is not
	// a verification: the edge may keep serving the old copy.
	verify?: { url: string; status?: number; bodyStatus?: string };
}

// The edge states that mean "no stored copy was served for this request".
const EDGE_FRESH_STATES = new Set(['MISS', 'BYPASS', 'EXPIRED', 'DYNAMIC', 'REVALIDATED', 'UPDATED']);

// The purge is verified by reading the affected URL back through the edge.
// A `HIT` means the edge still holds the old copy, which is the one thing
// the purge exists to prevent. The body is checked too: a withdrawn version
// has to answer 410, a yanked version has to say so.
async function verifyPurgeEdge(verify: NonNullable<PurgeOptions['verify']>): Promise<boolean> {
	let res: Response;
	try {
		res = await fetch(verify.url, { signal: AbortSignal.timeout(10000) });
	} catch {
		return false;
	}
	const state = String(res.headers.get('cf-cache-status') ?? '').toUpperCase();
	if (state === 'HIT') return false;
	if (verify.status !== undefined && res.status !== verify.status) return false;
	if (verify.bodyStatus !== undefined) {
		let status = '';
		try {
			const body = (await res.json()) as { status?: unknown };
			status = typeof body.status === 'string' ? body.status : '';
		} catch {
			status = '';
		}
		if (status !== verify.bodyStatus) return false;
	}
	// No edge header at all and no 410 to read: there is nothing that says
	// the copy is gone, so the purge is not verified.
	if (!EDGE_FRESH_STATES.has(state) && res.status !== 410) return false;
	return true;
}

// The purge is configured when the zone and the token are both set. Without
// them nothing is purged and the origin stays the only authority.
export function purgeConfigured(env: Record<string, string | undefined>): boolean {
	return Boolean(env['CF_ZONE_ID'] && env['CF_PURGE_TOKEN']);
}

export async function purgeUrls(
	env: Record<string, string | undefined>,
	urls: string[],
	options?: PurgeOptions
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
				if (res.ok && data?.success === true) {
					// The API answer is not the proof. The edge is read back
					// and has to answer with the new state.
					if (!options?.verify || (await verifyPurgeEdge(options.verify))) {
						return { ok: true, verified: true };
					}
				}
			} catch {
				// retried on the next attempt; the origin stays authoritative
			}
		}
		try {
			const db = await getDdb(env);
			if (!db) return { ok: false, verified: false };
			await db.insert(s.auditLog).values({
				action: 'purge-failed',
				fullName: options?.fullName ?? '',
				version: '',
				detailsJson: JSON.stringify({
					urls,
					prefixes: options?.prefixes ?? [],
					verified: false,
					verify: options?.verify ?? null
				}),
				createdAt: Math.floor(Date.now() / 1000)
			});
		} catch {
			// audit write is best-effort too
		}
		return { ok: false, verified: false };
	}
	try {
		const db = await getDdb(env);
		if (!db) return { ok: false, verified: false };
		await db.insert(s.auditLog).values({
			action: 'purge-skipped',
			fullName: options?.fullName ?? '',
			version: '',
			detailsJson: JSON.stringify({
				urls,
				prefixes: options?.prefixes ?? [],
				verified: false,
				reason: 'no purge credentials configured'
			}),
			createdAt: Math.floor(Date.now() / 1000)
		});
		return { ok: true, verified: false };
	} catch {
		return { ok: false, verified: false };
	}
}

export function packagePointerUrls(origin: string, full: string, version?: string): string[] {
	const base = origin.replace(/\/$/, '');
	const urls = [`${base}/api/v0/packages`, `${base}/api/v0/packages/${full}`];
	if (version) {
		urls.push(`${base}/api/v0/packages/${full}@${version}`);
		urls.push(`${base}/api/v0/packages/${full}@${version}/api`);
		urls.push(`${base}/api/v0/packages/${full}@${version}/manifest`);
		urls.push(`${base}/api/v0/packages/${full}@${version}/chunks`);
		urls.push(`${base}/api/v0/packages/${full}@${version}/guides`);
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
	artifactDigest?: string | null;
	entries?: TarEntryRef[];
}

export interface HaveEntry {
	full: string;
	version: string;
	integrity: string;
}

// The `have` list is indexed by name and version. The linear scan it
// replaces cost one pass over the list per node, so the cost of a resolve
// grew with nodes times list length; the index makes it one pass over the
// list and one lookup per node.
export type HaveIndex = Map<string, Set<string>>;

export function indexHave(have: HaveEntry[]): HaveIndex {
	const index: HaveIndex = new Map();
	for (const h of have) {
		const key = `${h.full}@${h.version}`;
		let digests = index.get(key);
		if (!digests) {
			digests = new Set<string>();
			index.set(key, digests);
		}
		digests.add(h.integrity);
	}
	return index;
}

// The counters of the last resolve. They exist so a test can assert the
// shape of the work a resolve did: how many `have` entries it compared
// and how many store round trips it spent.
export const resolveCounters = {
	haveComparisons: 0,
	subrequests: 0
};

export function resetResolveCounters(): void {
	resolveCounters.haveComparisons = 0;
	resolveCounters.subrequests = 0;
}

export function pruneHit(
	have: HaveEntry[] | HaveIndex,
	full: string,
	version: string,
	sha: string
): boolean {
	if (!sha) return false;
	if (have instanceof Map) {
		resolveCounters.haveComparisons += 1;
		const digests = have.get(`${full}@${version}`);
		return digests !== undefined && digests.has(sha);
	}
	resolveCounters.haveComparisons += have.length;
	return have.some((h) => h.full === full && h.version === version && h.integrity === sha);
}

// Section 16 budgets. A resolve is one HTTP request with a fixed subrequest
// ceiling, so an unbounded closure has to fail with a named error instead of
// a hang or a 500.
export const RESOLVE_MAX_DEPTH = 64;
export const RESOLVE_MAX_NODES = 512;
export const RESOLVE_MAX_FRONTIER = 512;
export const RESOLVE_MAX_CANDIDATES = 4096;
// The external subrequests one resolve may spend. Every store round trip is
// one, and the platform allows 50 per invocation on the Free plan, so a
// resolve that needs more refuses itself before the platform would kill
// the request. A deployment on a bigger plan raises the ceiling.
export const RESOLVE_MAX_SUBREQUESTS = 50;
// How often a resolve may re-read the candidates of a package whose answer
// could not be trusted: a stale memo list, a missing row, a withdrawn row.
// The bound is what turns an endless race into a named failure.
export const RESOLVE_REPAIR_ROUNDS = 2;
export const RESOLVE_MAX_INLINE_BYTES = 256 * 1024;
// A closure bigger than this is not an inline candidate. The file index of
// every package in the closure has to be read to answer an inline request,
// and a client that gets nothing inlined falls back to one manifest read
// per package, so a huge closure is refused before any index is read.
export const RESOLVE_MAX_INLINE_NODES = 256;
// Bound parameters per statement are capped at 900. SQLite's
// SQLITE_MAX_VARIABLE_NUMBER defaults to 32766 since 3.32.0, and Turso
// runs a modern SQLite, so 900 is far inside the real limit; the old 100
// was a self-imposed constant with a wrong comment. OR-expanded name
// lists are chunked to stay under it (2 params per name, 3 per pick).
const RESOLVE_SQL_MAX_PARAMS = 900;

export const VERSION_INDEX_TTL_MS = 600_000;
// The memo is a memory bound, not a correctness mechanism: an entry is
// evicted once the total number of memoized version strings passes this
// cap, and its age only decides when it is dropped.
const VERSION_INDEX_MAX_VERSION_STRINGS = 50_000;

// Isolate-scope memo of candidate version lists, keyed by package name.
//
// An entry is only a cache of a read, never a source of truth: it carries
// the per-package mutation count that was observed when the list was read,
// and a resolve may use the list only when the counter it reads back from
// the picked row still matches. A counter that moved means the candidate
// set changed, and the list is re-read.
interface VersionIndexEntry {
	at: number;
	token: number;
	versions: string[];
}

const versionIndex = new Map<string, VersionIndexEntry>();
let versionIndexVersionStrings = 0;

export function versionIndexEntries(): number {
	return versionIndex.size;
}

export function versionIndexSize(): number {
	return versionIndexVersionStrings;
}

export function clearVersionIndex(): void {
	versionIndex.clear();
	versionIndexVersionStrings = 0;
}

function versionIndexGet(full: string): VersionIndexEntry | null {
	const hit = versionIndex.get(full);
	if (!hit) return null;
	if (Date.now() - hit.at >= VERSION_INDEX_TTL_MS) {
		versionIndexDrop(full);
		return null;
	}
	return hit;
}

function versionIndexDrop(full: string): void {
	const hit = versionIndex.get(full);
	if (!hit) return;
	versionIndexVersionStrings -= hit.versions.length;
	versionIndex.delete(full);
}

function versionIndexSet(full: string, versions: string[], token: number): void {
	versionIndexDrop(full);
	versionIndex.set(full, { at: Date.now(), token, versions });
	versionIndexVersionStrings += versions.length;
	while (versionIndexVersionStrings > VERSION_INDEX_MAX_VERSION_STRINGS) {
		const oldest = versionIndex.keys().next();
		if (oldest.done) break;
		versionIndexDrop(oldest.value);
	}
}

function noteMutation(): void {
	clearVersionIndex();
}

function budgetError(what: string): Error {
	return new Error(`resolve budget exceeded: ${what}`);
}

// The subrequest budget of one resolve. A store round trip is one
// subrequest, and the platform kills an invocation that spends more than
// the plan allows. Spending the budget here means the refusal is a named
// error the route can answer, instead of a dead request.
class SubrequestBudget {
	private used = 0;

	constructor(private readonly limit: number) {}

	spend(count: number, what: string): void {
		if (this.used + count > this.limit) {
			throw budgetError(`external subrequests over ${this.limit} (${what})`);
		}
		this.used += count;
		resolveCounters.subrequests += count;
	}
}

function resolveSubrequestLimit(env: Record<string, string | undefined>): number {
	const raw = Number(env['RESOLVE_MAX_SUBREQUESTS'] ?? '');
	if (Number.isFinite(raw) && raw > 0) return Math.floor(raw);
	return RESOLVE_MAX_SUBREQUESTS;
}

interface ResolveRow {
	version: string;
	status: string;
	manifestDigest: string;
	artifactDigest: string;
	artifactMeta: string;
	engineRange: string;
	manifestJson: string;
	withdrawn: boolean;
	// The per-package mutation count read with the row: the proof that the
	// candidate list this pick was made from still describes the package.
	mutationCount: number;
}

// A version that a takedown reached is never served, not through a stale
// candidate list and not through a client `have` entry that names it.
function isWithdrawn(row: ResolveRow): boolean {
	return row.withdrawn || row.status === 'tombstoned';
}

const CANDIDATE_SQL_HEAD = `SELECT p.scope AS scope, p.name AS name, v.version AS version,
	p.mutation_count AS mutation_count
	FROM package_versions v JOIN packages p ON p.id = v.package_id
	WHERE v.status IN ('live', 'yanked') AND (`;
const CANDIDATE_SQL_TAIL = `)
	ORDER BY p.scope, p.name, ${SEMVER_ORDER}`;

function pickedRowSql(): string {
	// doc_json, guides_json, checksum, request_id and created_at are selected
	// by getVersionRow for the /api routes and are not read here; dropping them
	// keeps up to 1 MiB of jsdoc per package off the resolve read.
	//
	// tar_manifest_json is not selected here either: the file index is only
	// needed for an inline answer, and it is read in a second, bounded phase
	// once the closure is known (see attachInlineEntries).
	return `SELECT p.scope AS scope, p.name AS name,
	v.version AS version, v.status AS status, v.manifest_digest AS manifest_digest,
	v.artifact_digest AS artifact_digest, v.artifact_meta AS artifact_meta,
	v.engine_range AS engine_range, v.manifest_json AS manifest_json,
	p.mutation_count AS mutation_count,
	(SELECT 1 FROM tombstones t WHERE t.package_id = p.id AND t.version = v.version) AS withdrawn
	FROM package_versions v JOIN packages p ON p.id = v.package_id
	WHERE `;
}

// The candidate list of one package, with the mutation count observed while
// reading it. The count is the proof that the list is still current.
interface CandidateList {
	versions: string[];
	token: number;
}

// A package with no row carries no count: the list is empty and the answer
// for it is always read fresh.
const NO_TOKEN = -1;

async function resolveCandidates(
	raw: Client | null,
	fulls: string[],
	budget: SubrequestBudget
): Promise<Map<string, CandidateList>> {
	const out = new Map<string, CandidateList>();
	const want: Array<{ full: string; scope: string; name: string }> = [];
	for (const full of fulls) {
		const parsed = parsePackageName(full);
		// An unparseable name has no packages row, which is the same answer as
		// an empty candidate list and keeps `package X does not exist` intact.
		if (parsed) want.push({ full, scope: parsed.scope, name: parsed.name });
		else out.set(full, { versions: [], token: NO_TOKEN });
	}
	if (raw === null || want.length === 0) {
		for (const w of want) out.set(w.full, { versions: [], token: NO_TOKEN });
		return out;
	}
	const groups = chunked(want, Math.max(1, Math.floor(RESOLVE_SQL_MAX_PARAMS / 2)));
	budget.spend(1, 'candidate list');
	const results = await raw.batch(
		groups.map((group) => ({
			sql: `${CANDIDATE_SQL_HEAD}${group.map(() => '(p.scope = ? AND p.name = ?)').join(' OR ')}${CANDIDATE_SQL_TAIL}`,
			args: group.flatMap((w) => [w.scope, w.name])
		}))
	);
	for (let i = 0; i < groups.length; i++) {
		for (const row of results[i].rows) {
			const scope = String(row['scope'] ?? '');
			const name = String(row['name'] ?? '');
			const full = scope ? `@${scope}/${name}` : name;
			const entry = out.get(full) ?? { versions: [], token: NO_TOKEN };
			entry.versions.push(String(row['version']));
			// Every row of a package carries the count of its packages row.
			entry.token = Number(row['mutation_count'] ?? 0);
			out.set(full, entry);
		}
	}
	for (const w of want) if (!out.has(w.full)) out.set(w.full, { versions: [], token: NO_TOKEN });
	return out;
}

async function resolvePickedRows(
	raw: Client | null,
	picks: Array<{ full: string; version: string }>,
	budget: SubrequestBudget
): Promise<Map<string, ResolveRow>> {
	const out = new Map<string, ResolveRow>();
	if (raw === null || picks.length === 0) return out;
	const groups = chunked(picks, Math.max(1, Math.floor(RESOLVE_SQL_MAX_PARAMS / 3)));
	const head = pickedRowSql();
	budget.spend(1, 'picked rows');
	const results = await raw.batch(
		groups.map((group) => ({
			sql: `${head}${group.map(() => '(p.scope = ? AND p.name = ? AND v.version = ?)').join(' OR ')}`,
			args: group.flatMap((p) => {
				const parsed = parsePackageName(p.full);
				return [parsed?.scope ?? '', parsed?.name ?? p.full, p.version];
			})
		}))
	);
	for (let i = 0; i < groups.length; i++) {
		for (const row of results[i].rows) {
			const scope = String(row['scope'] ?? '');
			const name = String(row['name'] ?? '');
			const full = scope ? `@${scope}/${name}` : name;
			const version = String(row['version'] ?? '');
			out.set(`${full}@${version}`, {
				version,
				status: String(row['status'] ?? 'live'),
				manifestDigest: String(row['manifest_digest'] ?? ''),
				artifactDigest: String(row['artifact_digest'] ?? ''),
				artifactMeta: String(row['artifact_meta'] ?? ''),
				engineRange: String(row['engine_range'] ?? ''),
				manifestJson: String(row['manifest_json'] ?? '{}'),
				withdrawn: row['withdrawn'] !== null && row['withdrawn'] !== undefined,
				mutationCount: Number(row['mutation_count'] ?? 0)
			});
		}
	}
	return out;
}

// The file index of a version is read in two steps: a length probe first, so
// a closure that cannot fit the response cap is refused without the index
// ever leaving the database, and the index itself only for a closure that
// fits.
//
// The probe is `length(index)`, which SQLite answers from the record header
// alone (the column is read with the length argument flag, so the index
// pages are never touched). Casting the index to a blob would give a byte
// count, but it would also make SQLite read every byte of it, which is the
// cost this probe exists to avoid.
const MANIFEST_SIZE_SQL = `SELECT p.scope AS scope, p.name AS name, v.version AS version,
	length(v.tar_manifest_json) AS manifest_size
	FROM package_versions v JOIN packages p ON p.id = v.package_id
	WHERE `;

const MANIFEST_JSON_SQL = `SELECT p.scope AS scope, p.name AS name, v.version AS version,
	v.tar_manifest_json AS tar_manifest_json
	FROM package_versions v JOIN packages p ON p.id = v.package_id
	WHERE `;

function manifestPicksSql(
	picks: Array<{ full: string; version: string }>
): Array<{ sql: string; args: string[] }> {
	return chunked(picks, Math.max(1, Math.floor(RESOLVE_SQL_MAX_PARAMS / 3))).map((group) => ({
		sql: group.map(() => '(p.scope = ? AND p.name = ? AND v.version = ?)').join(' OR '),
		args: group.flatMap((p) => {
			const parsed = parsePackageName(p.full);
			return [parsed?.scope ?? '', parsed?.name ?? p.full, p.version];
		})
	}));
}

function pickedFull(row: Record<string, unknown>): string {
	const scope = String(row['scope'] ?? '');
	const name = String(row['name'] ?? '');
	return scope ? `@${scope}/${name}` : name;
}

async function resolveManifestSizes(
	raw: Client | null,
	picks: Array<{ full: string; version: string }>,
	budget: SubrequestBudget
): Promise<Map<string, number>> {
	const out = new Map<string, number>();
	if (raw === null || picks.length === 0) return out;
	const groups = manifestPicksSql(picks);
	budget.spend(1, 'file index sizes');
	const results = await raw.batch(
		groups.map((group) => ({ sql: `${MANIFEST_SIZE_SQL}${group.sql}`, args: group.args }))
	);
	for (const result of results) {
		for (const row of result.rows) {
			const full = pickedFull(row);
			out.set(`${full}@${String(row['version'] ?? '')}`, Number(row['manifest_size'] ?? 0));
		}
	}
	return out;
}

async function resolveManifestJson(
	raw: Client | null,
	picks: Array<{ full: string; version: string }>,
	budget: SubrequestBudget
): Promise<Map<string, string>> {
	const out = new Map<string, string>();
	if (raw === null || picks.length === 0) return out;
	const groups = manifestPicksSql(picks);
	budget.spend(1, 'file index');
	const results = await raw.batch(
		groups.map((group) => ({ sql: `${MANIFEST_JSON_SQL}${group.sql}`, args: group.args }))
	);
	for (const result of results) {
		for (const row of result.rows) {
			const full = pickedFull(row);
			out.set(`${full}@${String(row['version'] ?? '')}`, String(row['tar_manifest_json'] ?? '[]'));
		}
	}
	return out;
}

function manifestDeps(row: ResolveRow): Array<[string, string]> {
	const out: Array<[string, string]> = [];
	let deps: Record<string, unknown> = {};
	try {
		const manifest = JSON.parse(row.manifestJson) as Record<string, unknown>;
		if (manifest['deps'] && typeof manifest['deps'] === 'object') {
			deps = manifest['deps'] as Record<string, unknown>;
		}
	} catch {
		deps = {};
	}
	for (const [name, depRange] of Object.entries(deps)) out.push([name, String(depRange)]);
	return out;
}

function resolveEntries(text: string): TarEntryRef[] | null {
	try {
		const value = JSON.parse(text) as unknown;
		return Array.isArray(value) ? (value as TarEntryRef[]) : null;
	} catch {
		return null;
	}
}

export interface ResolveOptions {
	inlineChunks?: boolean;
	advertiseChunks?: boolean;
}

// Why an inline answer was refused. The client falls back to one manifest
// read per package, so it has to know that the refusal was a decision and
// not a registry without file indexes.
export type InlineDecline = 'closure-too-large' | 'size-cap-exceeded';

// The inline answer is decided from the closure, not from the rows that were
// read for it. The closure is resolved first with the cheap projection, then
// the file indexes are measured, and only a closure that fits the cap is
// read. A request that cannot fit therefore never pays for an index it
// would have to throw away.
async function attachInlineEntries(
	raw: Client | null,
	nodes: ResolveNode[],
	levels: ResolveNode[][],
	resolved: Record<string, string>,
	acceptedChunks: string | null,
	budget: SubrequestBudget
): Promise<InlineDecline | null> {
	if (nodes.length > RESOLVE_MAX_INLINE_NODES) return 'closure-too-large';
	const picks = nodes.map((n) => ({ full: n.full, version: n.version }));
	const sizes = await resolveManifestSizes(raw, picks, budget);
	let total = 0;
	for (const p of picks) total += sizes.get(`${p.full}@${p.version}`) ?? 0;
	// The stored index is a lower bound of the response: the response also
	// carries the per-node wrapper and every node's metadata, so a closure
	// whose stored index alone is over the cap cannot fit and is refused
	// here, before the index is read.
	if (total > RESOLVE_MAX_INLINE_BYTES) return 'size-cap-exceeded';
	const manifests = await resolveManifestJson(raw, picks, budget);
	for (const node of nodes) {
		const text = manifests.get(`${node.full}@${node.version}`);
		if (text === undefined) continue;
		const entries = resolveEntries(text);
		if (entries) node.entries = entries;
	}
	// The exact measurement, and the one the cap is about: the size of the
	// resolve body the client receives, with the inline entries in it.
	if (resolveResponseBytes(resolved, levels, acceptedChunks) > RESOLVE_MAX_INLINE_BYTES) {
		for (const node of nodes) node.entries = undefined;
		return 'size-cap-exceeded';
	}
	return null;
}

// The inline budget is measured against the response the client receives,
// not against the stored file indexes.
//
// The stored index (`tar_manifest_json`) is what the server kept; the
// response is what crosses the wire, and it carries the per-node
// `{"entries": ...}` wrapper and every node's metadata on top of the index
// bytes. Measuring the stored strings undercounted the response by about
// 44 %, so the cap did not mean what it said. The measurement is now the
// serialized size of the whole resolve body, byte for byte, with the inline
// entries in it - the same body `api/v0/resolve/+server.ts` sends, so the two
// must stay in step.
function resolveResponseBytes(
	resolved: Record<string, string>,
	levels: ResolveNode[][],
	chunks: string | null
): number {
	const payload: Record<string, unknown> = { resolved, base: '', levels };
	if (chunks) payload['chunks'] = chunks;
	return new TextEncoder().encode(JSON.stringify(payload)).length;
}

// Frontier expansion is level-batched: one batched IN-statement for the
// candidate versions and one for the picked rows per level, so a 17-package
// resolve costs two store round trips instead of 35. The batched pass only
// gathers rows; picking, conflict detection, cycle surfacing and node order
// all run in the same DFS replay the serial version used, so `resolved`,
// `levels` and every error string are unchanged.
export async function resolveGraph(
	env: Record<string, string | undefined>,
	requirements: Record<string, string>,
	have: HaveEntry[],
	options: ResolveOptions = {}
): Promise<{
	resolved: Record<string, string>;
	levels: ResolveNode[][];
	chunks: string | null;
	chunksDeclined: InlineDecline | null;
}> {
	const inline = options.inlineChunks === true;
	const budget = new SubrequestBudget(resolveSubrequestLimit(env));
	// The schema check of a cold isolate is one round trip of its own.
	budget.spend(1, 'schema check');
	await getDdb(env);
	const raw = getClient(env);
	// The have list is indexed once: the prune lookup is then one map read
	// per node instead of one pass over the list per node.
	const haveIndex = indexHave(have);

	const roots = Object.entries(requirements).map(([full, range]) => [full, String(range)] as const);
	for (const [full] of roots) {
		if (!parsePackageName(full)) throw new Error(`invalid package name: ${full}`);
	}

	const candidates = new Map<string, string[]>();
	const rows = new Map<string, ResolveRow>();
	const depCache = new Map<string, Array<[string, string]>>();
	const ranges = new Map<string, string[]>();
	const blocked = new Set<string>();

	const depsOf = (id: string, row: ResolveRow): Array<[string, string]> => {
		let cached = depCache.get(id);
		if (!cached) {
			cached = manifestDeps(row);
			depCache.set(id, cached);
		}
		return cached;
	};

	const addRange = (full: string, range: string): boolean => {
		if (blocked.has(full)) return false;
		const list = ranges.get(full);
		if (list) {
			list.push(range);
			return false;
		}
		ranges.set(full, [range]);
		return true;
	};

	for (const [full, range] of roots) addRange(full, range);
	let frontier = [...ranges.keys()].sort();
	let depth = 0;
	let visited = 0;

	while (frontier.length > 0) {
		depth += 1;
		if (depth > RESOLVE_MAX_DEPTH) throw budgetError(`traversal depth over ${RESOLVE_MAX_DEPTH}`);
		if (frontier.length > RESOLVE_MAX_FRONTIER) {
			throw budgetError(`frontier width over ${RESOLVE_MAX_FRONTIER}`);
		}

		// The candidate lists of this level. A list comes either from the
		// memo or from a read in this request. The memo carries the mutation
		// count it was built with, and that count is what makes the list
		// usable: a list is only trusted while the count still matches.
		const memoHit: Array<[string, VersionIndexEntry]> = [];
		const missing: string[] = [];
		for (const full of frontier) {
			if (candidates.has(full)) continue;
			const hit = versionIndexGet(full);
			if (hit) memoHit.push([full, hit]);
			else missing.push(full);
		}
		const tokens = new Map<string, number>();
		const fromMemo = new Set<string>();
		if (missing.length > 0) {
			const fresh = await resolveCandidates(raw, missing, budget);
			for (const [full, list] of fresh) {
				// The candidate list is copied: the memo keeps its own copy.
				candidates.set(full, [...list.versions]);
				tokens.set(full, list.token);
				// An empty list is not memoized. It is the answer for a name
				// that does not exist, and that answer is always read fresh.
				if (list.versions.length > 0) versionIndexSet(full, list.versions, list.token);
			}
		}
		for (const [full, hit] of memoHit) {
			candidates.set(full, [...hit.versions]);
			tokens.set(full, hit.token);
			fromMemo.add(full);
		}

		let candidateCount = 0;
		for (const full of frontier) candidateCount += candidates.get(full)?.length ?? 0;
		if (candidateCount > RESOLVE_MAX_CANDIDATES) {
			throw budgetError(`candidate count over ${RESOLVE_MAX_CANDIDATES}`);
		}

		// Every accumulated range's pick is loaded, not just the first one:
		// the replay picks with whichever range it reaches first, and that
		// range is always already known here because its parent sits one level
		// closer to the roots.
		const picksByName = new Map<string, string[]>();
		const wanted: Array<{ full: string; version: string }> = [];
		const pickFor = (full: string): string[] => {
			const versions = candidates.get(full) ?? [];
			const picked: string[] = [];
			for (const range of ranges.get(full) ?? []) {
				const pick = maxSatisfying(versions, range);
				if (!pick || picked.includes(pick)) continue;
				picked.push(pick);
			}
			return picked;
		};
		for (const full of frontier) {
			const picked = pickFor(full);
			picksByName.set(full, picked);
			for (const version of picked) wanted.push({ full, version });
		}

		// A package that produced no pick is answered from a fresh read: the
		// memo is never the source of "does not exist" or "no version
		// satisfies". A list that was read in this request is already fresh,
		// so only a memo list is re-read.
		const unpicked = frontier.filter(
			(full) => fromMemo.has(full) && (picksByName.get(full) ?? []).length === 0
		);
		if (unpicked.length > 0) {
			const fresh = await resolveCandidates(raw, unpicked, budget);
			for (const [full, list] of fresh) {
				candidates.set(full, [...list.versions]);
				tokens.set(full, list.token);
				fromMemo.delete(full);
				if (list.versions.length > 0) versionIndexSet(full, list.versions, list.token);
				else versionIndexDrop(full);
			}
			for (const full of unpicked) {
				const picked = pickFor(full);
				picksByName.set(full, picked);
				for (const version of picked) wanted.push({ full, version });
			}
		}
		for (const full of frontier) {
			if ((picksByName.get(full) ?? []).length === 0) blocked.add(full);
		}

		const picked = await resolvePickedRows(raw, wanted, budget);
		for (const [id, row] of picked) rows.set(id, row);

		// The repair set holds the packages whose answer cannot be trusted
		// yet: a pick whose row is gone (a delete or a transfer moved it), a
		// pick that a takedown reached, and a memo list whose package changed
		// since the list was read (the mutation count moved).
		const repair = new Set<string>();
		for (const w of wanted) {
			const row = picked.get(`${w.full}@${w.version}`);
			if (!row || isWithdrawn(row)) {
				repair.add(w.full);
				continue;
			}
			if (fromMemo.has(w.full) && row.mutationCount !== tokens.get(w.full)) repair.add(w.full);
		}

		// The repair re-reads the candidates of the affected packages, picks
		// again and reads the rows again. It is bounded: a registry that
		// keeps racing answers "resolve raced" instead of a wrong version.
		let pending = [...repair].sort();
		for (let round = 0; round < RESOLVE_REPAIR_ROUNDS && pending.length > 0; round++) {
			const fresh = await resolveCandidates(raw, pending, budget);
			const again: Array<{ full: string; version: string }> = [];
			for (const full of pending) {
				const list = fresh.get(full) ?? { versions: [], token: NO_TOKEN };
				candidates.set(full, [...list.versions]);
				tokens.set(full, list.token);
				fromMemo.delete(full);
				if (list.versions.length > 0) versionIndexSet(full, list.versions, list.token);
				else versionIndexDrop(full);
				const pickedAgain = pickFor(full);
				picksByName.set(full, pickedAgain);
				for (const version of pickedAgain) again.push({ full, version });
			}
			for (const [id, row] of await resolvePickedRows(raw, again, budget)) rows.set(id, row);
			const still: string[] = [];
			for (const full of pending) {
				const versions = picksByName.get(full) ?? [];
				if (versions.length === 0) {
					// The package has no version left. The answer is an
					// honest "does not exist" or "no version satisfies".
					blocked.add(full);
					continue;
				}
				for (const version of versions) {
					const row = rows.get(`${full}@${version}`);
					if (!row || isWithdrawn(row)) {
						still.push(full);
						break;
					}
				}
			}
			pending = still;
		}
		if (pending.length > 0) throw new Error('resolve raced with a mutation');
		// A repair may have read a longer candidate list than the one the
		// budget was checked against.
		let repairedCount = 0;
		for (const full of frontier) repairedCount += candidates.get(full)?.length ?? 0;
		if (repairedCount > RESOLVE_MAX_CANDIDATES) {
			throw budgetError(`candidate count over ${RESOLVE_MAX_CANDIDATES}`);
		}

		const next: string[] = [];
		for (const full of frontier) {
			visited += 1;
			if (visited > RESOLVE_MAX_NODES) throw budgetError(`closure over ${RESOLVE_MAX_NODES} nodes`);
			for (const version of picksByName.get(full) ?? []) {
				const id = `${full}@${version}`;
				const row = rows.get(id);
				if (!row) continue;
				if (pruneHit(haveIndex, full, version, row.manifestDigest)) continue;
				for (const [name, range] of depsOf(id, row)) {
					if (addRange(name, range)) next.push(name);
				}
			}
		}
		frontier = next.sort();
	}

	const makeNode = (full: string, version: string, row: ResolveRow, deps: string[]): ResolveNode => {
		const node: ResolveNode = {
			full,
			version,
			path: `${full}@${version}/chunks`,
			integrity: row.manifestDigest,
			engineRange: row.engineRange,
			deps,
			yanked: row.status === 'yanked'
		};
		if (row.artifactDigest !== '') node.artifactDigest = row.artifactDigest;
		return node;
	};

	const memo = new Map<string, ResolveNode>();
	const resolved: Record<string, string> = {};

	const visit = (full: string, range: string): string => {
		const known = resolved[full];
		if (known) {
			if (!satisfiesRange(known, range)) {
				throw new Error(`conflicting ranges for ${full}: ${known} does not satisfy ${range}`);
			}
			return known;
		}
		const versions = candidates.get(full) ?? [];
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
		const row = rows.get(id);
		if (!row) throw new Error(`package ${id} does not exist`);
		if (pruneHit(haveIndex, full, pick, row.manifestDigest)) {
			memo.set(id, makeNode(full, pick, row, []));
			return pick;
		}
		const depNames: string[] = [];
		for (const [name, depRange] of depsOf(id, row)) {
			depNames.push(name);
			visit(name, depRange);
		}
		memo.set(id, makeNode(full, pick, row, depNames));
		return pick;
	};

	for (const [full, range] of roots) visit(full, range);
	const nodes = [...memo.values()];
	const byId = new Map(nodes.map((n) => [`${n.full}@${n.version}`, n]));
	const order = levelize(nodes.map((n) => ({
		id: `${n.full}@${n.version}`,
		deps: n.deps.map((d) => {
			const v = resolved[d];
			return v ? `${d}@${v}` : d;
		})
	})));

	// Inline is a second, bounded phase. The closure above was resolved with
	// the cheap projection, so a request that cannot fit the cap never paid
	// for the file indexes it would have had to throw away. The client is
	// told that the inline answer was declined, so it can fall back to one
	// manifest read per package instead of wondering why the body changed.
	let chunks: string | null = options.advertiseChunks === true ? 'links' : null;
	let chunksDeclined: InlineDecline | null = null;
	const levels = order.map((ids) => ids.map((id) => byId.get(id)!));
	if (inline) {
		const declined = await attachInlineEntries(
			raw,
			nodes,
			levels,
			resolved,
			options.advertiseChunks === true ? 'inline' : null,
			budget
		);
		if (declined) chunksDeclined = declined;
		else if (options.advertiseChunks === true) chunks = 'inline';
	}

	return {
		resolved,
		levels,
		chunks,
		chunksDeclined
	};
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
	const db = await getDdb(env);
	if (!db) throw new Error('missing database');
	const now = Math.floor(Date.now() / 1000);
	const res = await db.insert(s.benchmarkRuns).values({
		githubRunId,
		commitSha,
		snapshotJson,
		createdAt: now
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
	const db = await getDdb(env);
	if (!db) return null;
	const b = s.benchmarkRuns;
	const row = await db
		.select({
			id: b.id,
			githubRunId: b.githubRunId,
			commitSha: b.commitSha,
			snapshotJson: b.snapshotJson,
			createdAt: b.createdAt
		})
		.from(b)
		.orderBy(desc(b.id))
		.limit(1)
		.get();
	if (!row) return null;
	return {
		id: row.id,
		githubRunId: row.githubRunId,
		commitSha: row.commitSha,
		snapshotJson: row.snapshotJson,
		createdAt: row.createdAt
	};
}

export async function listBenchmarkRuns(
	env: Record<string, string | undefined>,
	limit: number
): Promise<Omit<BenchmarkRun, 'snapshotJson'>[]> {
	const db = await getDdb(env);
	if (!db) return [];
	const b = s.benchmarkRuns;
	const rows = await db
		.select({ id: b.id, githubRunId: b.githubRunId, commitSha: b.commitSha, createdAt: b.createdAt })
		.from(b)
		.orderBy(desc(b.id))
		.limit(Math.max(1, Math.min(60, Math.floor(limit) || 12)));
	return rows.map((r) => ({
		id: r.id,
		githubRunId: r.githubRunId,
		commitSha: r.commitSha,
		createdAt: r.createdAt
	}));
}

export async function isVersionWithdrawn(
	env: Record<string, string | undefined>,
	full: string,
	version: string
): Promise<boolean> {
	const parsed = parsePackageName(full);
	const db = await getDdb(env);
	if (!parsed || !db) return false;
	const t = s.tombstones;
	const p = s.packages;
	const row = await db
		.select({ one: sql`1` })
		.from(t)
		.innerJoin(p, eq(p.id, t.packageId))
		.where(and(eq(p.scope, parsed.scope), eq(p.name, parsed.name), eq(t.version, version)))
		.get();
	return row !== undefined;
}

export async function versionReuseBlocked(
	env: Record<string, string | undefined>,
	full: string,
	version: string
): Promise<boolean> {
	const parsed = parsePackageName(full);
	const db = await getDdb(env);
	if (!parsed || !db) return false;
	const id = packageId(parsed.scope, parsed.name);
	const v = s.packageVersions;
	const live = await db
		.select({ one: sql`1` })
		.from(v)
		.where(and(eq(v.packageId, id), eq(v.version, version)))
		.get();
	if (live !== undefined) return true;
	if (await isVersionWithdrawn(env, full, version)) return true;
	const a = s.auditLog;
	const audit = await db
		.select({ one: sql`1` })
		.from(a)
		.where(
			and(
				eq(a.fullName, parsed.full),
				eq(a.version, version),
				inArray(a.action, ['takedown', 'transfer', 'special-delete', 'publish'])
			)
		)
		.get();
	return audit !== undefined;
}

export async function recordTombstone(
	env: Record<string, string | undefined>,
	full: string,
	version: string,
	reason: string
): Promise<void> {
	const parsed = parsePackageName(full);
	const db = await getDdb(env);
	if (!parsed || !db) throw new Error('invalid package name or missing database');
	const now = Math.floor(Date.now() / 1000);
	const id = packageId(parsed.scope, parsed.name);
	const cur = await db
		.select({ manifestJson: s.packageVersions.manifestJson })
		.from(s.packageVersions)
		.where(and(eq(s.packageVersions.packageId, id), eq(s.packageVersions.version, version)))
		.get();
	let manifestJson = cur?.manifestJson ?? '{}';
	try {
		const manifest = JSON.parse(manifestJson) as Record<string, unknown>;
		manifest['withdrawReason'] = reason;
		manifestJson = JSON.stringify(manifest);
	} catch {
		manifestJson = JSON.stringify({ withdrawReason: reason });
	}
	await db.batch([
		db
			.insert(s.tombstones)
			.values({ packageId: id, version, reason, createdAt: now })
			.onConflictDoNothing(),
		db
			.update(s.packageVersions)
			.set({ status: 'tombstoned', manifestJson })
			.where(
				and(eq(s.packageVersions.packageId, id), eq(s.packageVersions.version, version))
			),
		db.insert(s.auditLog).values({
			action: 'takedown',
			fullName: parsed.full,
			version,
			detailsJson: JSON.stringify({ reason }),
			createdAt: now
		}),
		// A takedown takes the version out of the candidate set, so the
		// counter of this package moves with it. The row is written in the
		// same batch, so no resolve can see one without the other.
		db
			.update(s.packages)
			.set({ mutationCount: sql`${s.packages.mutationCount} + 1` })
			.where(eq(s.packages.id, id))
	] as unknown as [BatchItem<'sqlite'>, ...BatchItem<'sqlite'>[]]);
	noteMutation();
}
