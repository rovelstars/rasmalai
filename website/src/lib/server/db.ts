import { createClient, type Client } from '@libsql/client/web';
import { drizzle } from 'drizzle-orm/libsql/web';
import type { BatchItem } from 'drizzle-orm/batch';
import { alias } from 'drizzle-orm/sqlite-core';
import { and, asc, count, desc, eq, gte, inArray, lt, notExists, or, sql, sum } from 'drizzle-orm';
import * as s from './schema.js';
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

const SCHEMA_VERSION = 2;

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
	if (!names.has('license_id')) {
		await db.execute('ALTER TABLE packages ADD COLUMN license_id INTEGER REFERENCES licenses(id)');
	}
	await db.execute("UPDATE packages SET owner = 'org' WHERE owner = ''");
	// License normalization: keep the TEXT column during transition, but
	// point every row at its licenses FK entry.
	await db.execute(
		'INSERT INTO licenses (spdx) SELECT DISTINCT license FROM packages WHERE license IS NOT NULL ON CONFLICT(spdx) DO NOTHING'
	);
	await db.execute(
		'UPDATE packages SET license_id = (SELECT id FROM licenses WHERE spdx = packages.license) WHERE license_id IS NULL'
	);
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
	const db = await getDdb(env);
	if (!parsed || !db) throw new Error('invalid package name or missing database');
	const sem = parseSemver(p.version);
	if (!sem) throw new Error('invalid package version (expected X.Y.Z)');
	const now = Math.floor(Date.now() / 1000);
	const id = packageId(parsed.scope, parsed.name);
	const auditJson = JSON.stringify({ checksum: p.checksum });
	const keywords = (p.keywords ?? []).join(',');
	// Dependents reflect ever-depending packages (npm shows all): rows are
	// derived from manifests at publish time and are never deleted on
	// yank or tombstone, so no counter can desync under retries.
	const depNames = extractDepNames(p.manifestJson ?? '{}');
	const capabilities = p.capabilities ?? [];
	// Capability domains are static reference rows: ensure them first so the
	// version insert and its capability rows can land in one batch below.
	const domainIds = new Map<string, number>();
	if (capabilities.length > 0) {
		const names = [...new Set(capabilities.map((c) => c.domain))].sort();
		await db.batch(
			names.map((name) =>
				db.insert(s.capabilityDomains).values({ name }).onConflictDoNothing()
			) as unknown as [BatchItem<'sqlite'>, ...BatchItem<'sqlite'>[]]
		);
		const drows = await db
			.select({ id: s.capabilityDomains.id, name: s.capabilityDomains.name })
			.from(s.capabilityDomains)
			.where(inArray(s.capabilityDomains.name, names));
		for (const r of drows) domainIds.set(r.name, r.id);
	}
	let licenseId: number | null = null;
	if (p.license) {
		await db
			.insert(s.licenses)
			.values({ spdx: p.license })
			.onConflictDoNothing();
		const lrow = await db
			.select({ id: s.licenses.id })
			.from(s.licenses)
			.where(eq(s.licenses.spdx, p.license))
			.get();
		licenseId = lrow?.id ?? null;
	}
	const versionId = `ver_${id}_${p.version}`;
	// changes() reads the version insert directly above, so the audit row
	// lands only when this publish actually minted the version. The dep
	// inserts trail the audit row to keep that reading intact.
	const applied = await db.batch([
		db
			.insert(s.scopes)
			.values({
				name: parsed.scope,
				owner: ORG_OWNER,
				reserved: RESERVED_SCOPES.includes(parsed.scope) ? 1 : 0,
				createdAt: now
			})
			.onConflictDoNothing(),
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
				licenseId,
				downloads: 0,
				stars: 0,
				tags: p.tags.join(','),
				keywords,
				createdAt: now,
				updatedAt: now
			})
			.onConflictDoUpdate({
				target: [s.packages.scope, s.packages.name],
				set: {
					description: sql`excluded.description`,
					keywords: sql`excluded.keywords`,
					updatedAt: sql`excluded.updated_at`
				}
			}),
		db
			.insert(s.packageVersions)
			.values({
				id: versionId,
				packageId: id,
				version: p.version,
				readmeMarkdown: p.readme,
				docJson: p.docJson,
				checksum: p.checksum,
				status: 'live',
				createdAt: now,
				semverMajor: sem.major,
				semverMinor: sem.minor,
				semverPatch: sem.patch,
				prerelease: sem.prerelease,
				engineRange: p.engineRange ?? '',
				manifestJson: p.manifestJson ?? '{}',
				guidesJson: p.guidesJson ?? '[]',
				tarballSha256: p.tarballSha256 ?? '',
				requestId: p.requestId ?? ''
			})
			.onConflictDoNothing(),
		db.run(
			sql`INSERT INTO audit_log (action, full_name, version, details_json, request_id, created_at)
			      SELECT 'publish', ${parsed.full}, ${p.version}, ${auditJson}, ${p.requestId ?? ''}, ${now} WHERE changes() > 0`
		),
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
					domainId: domainIds.get(c.domain) ?? -1,
					arg: c.arg,
					reason: c.reason
				})
				.onConflictDoNothing()
		),
		// Appended last: the audit row above reads changes() from the version
		// insert at index 2, so the generation bump must not land before it.
		generationBump(db)
	] as unknown as [BatchItem<'sqlite'>, ...BatchItem<'sqlite'>[]]);
	const created = (applied[2]?.rowsAffected ?? 0) > 0;
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
	engineRange: string;
	docJson: string;
	manifestJson: string;
	guidesJson: string;
	tarManifestJson: string;
	requestId: string;
	createdAt: number;
}

export const SEMVER_ORDER = `semver_major DESC, semver_minor DESC, semver_patch DESC,
	CASE WHEN prerelease = '' THEN 1 ELSE 0 END DESC, prerelease DESC, v.created_at DESC`;

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
			id: v.id,
			version: v.version,
			status: v.status,
			checksum: v.checksum,
			tarballSha256: v.tarballSha256,
			tarManifestJson: v.tarManifestJson,
			engineRange: v.engineRange,
			docJson: v.docJson,
			manifestJson: v.manifestJson,
			guidesJson: v.guidesJson,
			requestId: v.requestId,
			createdAt: v.createdAt
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
		),
		generationBump(db)
	] as unknown as [BatchItem<'sqlite'>, ...BatchItem<'sqlite'>[]]);
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
				.set({ id: r.newId, scope: newOwner, updatedAt: now })
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
	stmts.push(generationBump(db));
	await db.batch(stmts as [BatchItem<'sqlite'>, ...BatchItem<'sqlite'>[]]);
	noteMutation();
}

export async function putChunk(
	env: Record<string, string | undefined>,
	hash: string,
	sizeBytes: number,
	bytes: Uint8Array
): Promise<void> {
	const db = await getDdb(env);
	if (!db) throw new Error('missing database');
	const bucket = r2Bucket(env);
	const now = Math.floor(Date.now() / 1000);
	if (bucket) {
		await db.run(
			sql`INSERT INTO chunks (hash, size_bytes, bytes, first_seen_at)
			      VALUES (${hash}, ${sizeBytes}, x'', ${now})
			      ON CONFLICT(hash) DO NOTHING`
		);
		await r2PutIfMissing(bucket, hash, bytes);
		return;
	}
	await db.run(
		sql`INSERT INTO chunks (hash, size_bytes, bytes, first_seen_at)
		      VALUES (${hash}, ${sizeBytes}, ${bytes}, ${now})
		      ON CONFLICT(hash) DO NOTHING`
	);
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
	const db = await getDdb(env);
	if (!db) throw new Error('missing database');
	if (hashes.length === 0) return;
	await db.batch(
		hashes.map((hash, ord) =>
			db.insert(s.manifestChunks).values({ chunkHash: hash, versionId: versionRowId, ord })
		) as unknown as [BatchItem<'sqlite'>, ...BatchItem<'sqlite'>[]]
	);
}

export async function storeTarManifest(
	env: Record<string, string | undefined>,
	versionRowId: string,
	entries: TarEntryRef[]
): Promise<void> {
	const db = await getDdb(env);
	if (!db) throw new Error('missing database');
	await db
		.update(s.packageVersions)
		.set({ tarManifestJson: JSON.stringify(entries) })
		.where(eq(s.packageVersions.id, versionRowId));
}

export async function findOrphanChunks(
	env: Record<string, string | undefined>,
	beforeUnix: number,
	limit: number
): Promise<string[]> {
	const db = await getDdb(env);
	if (!db) return [];
	const c = s.chunks;
	const mc = s.manifestChunks;
	const v = s.packageVersions;
	const rows = await db
		.select({ hash: c.hash })
		.from(c)
		.where(
			and(
				lt(c.firstSeenAt, beforeUnix),
				notExists(
					db
						.select({ one: sql`1` })
						.from(mc)
						.innerJoin(v, eq(v.id, mc.versionId))
						.where(and(eq(mc.chunkHash, c.hash), inArray(v.status, ['live', 'yanked'])))
				)
			)
		)
		.limit(Math.max(1, Math.min(1000, limit)));
	return rows.map((r) => r.hash);
}

export async function reverifyChunks(
	env: Record<string, string | undefined>,
	hashes: string[]
): Promise<Set<string>> {
	const db = await getDdb(env);
	if (!db || hashes.length === 0) return new Set();
	const mc = s.manifestChunks;
	const v = s.packageVersions;
	const live = new Set<string>();
	for (const batch of chunked(hashes, 50)) {
		const rows = await db
			.select({ hash: mc.chunkHash })
			.from(mc)
			.innerJoin(v, eq(v.id, mc.versionId))
			.where(and(inArray(mc.chunkHash, batch), inArray(v.status, ['live', 'yanked'])))
			.groupBy(mc.chunkHash);
		for (const r of rows) live.add(r.hash);
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
			const db = await getDdb(env);
			if (!db) return { ok: false, verified: false };
			await db.insert(s.auditLog).values({
				action: 'purge-failed',
				fullName: options?.fullName ?? '',
				version: '',
				detailsJson: JSON.stringify({ urls, prefixes: options?.prefixes ?? [] }),
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
			detailsJson: JSON.stringify({ urls, prefixes: options?.prefixes ?? [] }),
			createdAt: Math.floor(Date.now() / 1000)
		});
		return { ok: true, verified: false };
	} catch {
		return { ok: false, verified: false };
	}
}

export function packagePointerUrls(origin: string, full: string, version?: string): string[] {
	const base = origin.replace(/\/$/, '');
	const urls = [`${base}/api/packages`, `${base}/api/packages/${full}`, `${base}/packages`, `${base}/packages/${full}`];
	if (version) {
		urls.push(`${base}/api/packages/${full}@${version}`);
		urls.push(`${base}/api/packages/${full}@${version}/api`);
		urls.push(`${base}/api/packages/${full}@${version}/manifest`);
		urls.push(`${base}/api/packages/${full}@${version}/chunks`);
		urls.push(`${base}/api/packages/${full}@${version}/guides`);
		urls.push(`${base}/packages/${full}@${version}`);
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
	entries?: TarEntryRef[];
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

// Section 16 budgets. A resolve is one HTTP request with a fixed subrequest
// ceiling, so an unbounded closure has to fail with a named error instead of
// a hang or a 500.
export const RESOLVE_MAX_DEPTH = 64;
export const RESOLVE_MAX_NODES = 512;
export const RESOLVE_MAX_FRONTIER = 512;
export const RESOLVE_MAX_CANDIDATES = 4096;
export const RESOLVE_MAX_INLINE_BYTES = 256 * 1024;
// SQLite caps bound parameters per statement; OR-expanded name lists are
// chunked to stay under it (2 params per name, 3 per version pick).
const RESOLVE_SQL_MAX_PARAMS = 100;

export const GENERATION_KEY = 'registry_generation';
export const VERSION_INDEX_TTL_MS = 30_000;
const VERSION_INDEX_MAX = 4096;

// Bumped inside the same batch as every publish, yank, tombstone and
// transfer, so a mutation invalidates every generation-keyed artifact by
// construction and correctness never depends on the purge API answering.
const GENERATION_BUMP_SQL = `INSERT INTO meta (key, value) VALUES ('registry_generation', '1')
	ON CONFLICT(key) DO UPDATE SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)`;

function generationBump(db: Ddb): BatchItem<'sqlite'> {
	return db.run(sql.raw(GENERATION_BUMP_SQL)) as unknown as BatchItem<'sqlite'>;
}

export async function readRegistryGeneration(env: Record<string, string | undefined>): Promise<number> {
	const raw = getClient(env);
	if (!raw) return 0;
	try {
		const rs = await raw.execute({ sql: 'SELECT value FROM meta WHERE key = ?', args: [GENERATION_KEY] });
		const n = Number(rs.rows[0]?.['value'] ?? 0);
		return Number.isFinite(n) && n > 0 ? n : 0;
	} catch {
		return 0;
	}
}

// Isolate-scope memo of candidate version lists, keyed by package name. It
// cannot serve a stale withdrawal: it stores only the live+yanked version
// strings, while the chosen row and the tombstones subquery are read fresh on
// every request. A version yanked inside the TTL is still a candidate, the
// row read returns its yanked status, and a tombstone inside the TTL is
// caught by that subquery.
const versionIndex = new Map<string, { at: number; versions: string[] }>();

export function versionIndexEntries(): number {
	return versionIndex.size;
}

export function clearVersionIndex(): void {
	versionIndex.clear();
}

function versionIndexGet(full: string): string[] | null {
	const hit = versionIndex.get(full);
	if (!hit) return null;
	if (Date.now() - hit.at >= VERSION_INDEX_TTL_MS) {
		versionIndex.delete(full);
		return null;
	}
	return hit.versions;
}

function versionIndexSet(full: string, versions: string[]): void {
	versionIndex.delete(full);
	versionIndex.set(full, { at: Date.now(), versions });
	while (versionIndex.size > VERSION_INDEX_MAX) {
		const oldest = versionIndex.keys().next();
		if (oldest.done) break;
		versionIndex.delete(oldest.value);
	}
}

function noteMutation(): void {
	clearVersionIndex();
}

function budgetError(what: string): Error {
	return new Error(`resolve budget exceeded: ${what}`);
}

interface ResolveRow {
	version: string;
	status: string;
	tarballSha256: string;
	engineRange: string;
	manifestJson: string;
	tarManifestJson: string;
	withdrawn: boolean;
}

const CANDIDATE_SQL_HEAD = `SELECT p.scope AS scope, p.name AS name, v.version AS version
	FROM package_versions v JOIN packages p ON p.id = v.package_id
	WHERE v.status IN ('live', 'yanked') AND (`;
const CANDIDATE_SQL_TAIL = `)
	ORDER BY p.scope, p.name, ${SEMVER_ORDER}`;

function pickedRowSql(inline: boolean): string {
	// doc_json, guides_json, checksum, request_id and created_at are selected
	// by getVersionRow for the /api routes and are not read here; dropping them
	// keeps up to 1 MiB of jsdoc per package off the resolve read.
	return `SELECT p.scope AS scope, p.name AS name,
	v.version AS version, v.status AS status, v.tarball_sha256 AS tarball_sha256,
	v.engine_range AS engine_range, v.manifest_json AS manifest_json,
	(SELECT 1 FROM tombstones t WHERE t.package_id = p.id AND t.version = v.version) AS withdrawn${
		inline ? ', v.tar_manifest_json AS tar_manifest_json' : ''
	}
	FROM package_versions v JOIN packages p ON p.id = v.package_id
	WHERE `;
}

async function resolveCandidates(
	raw: Client | null,
	fulls: string[]
): Promise<Map<string, string[]>> {
	const out = new Map<string, string[]>();
	const want: Array<{ full: string; scope: string; name: string }> = [];
	for (const full of fulls) {
		const parsed = parsePackageName(full);
		// An unparseable name has no packages row, which is the same answer as
		// an empty candidate list and keeps `package X does not exist` intact.
		if (parsed) want.push({ full, scope: parsed.scope, name: parsed.name });
		else out.set(full, []);
	}
	if (raw === null || want.length === 0) {
		for (const w of want) out.set(w.full, []);
		return out;
	}
	const groups = chunked(want, Math.max(1, Math.floor(RESOLVE_SQL_MAX_PARAMS / 2)));
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
			const list = out.get(full) ?? [];
			list.push(String(row['version']));
			out.set(full, list);
		}
	}
	for (const w of want) if (!out.has(w.full)) out.set(w.full, []);
	return out;
}

async function resolvePickedRows(
	raw: Client | null,
	picks: Array<{ full: string; version: string }>,
	inline: boolean
): Promise<Map<string, ResolveRow>> {
	const out = new Map<string, ResolveRow>();
	if (raw === null || picks.length === 0) return out;
	const groups = chunked(picks, Math.max(1, Math.floor(RESOLVE_SQL_MAX_PARAMS / 3)));
	const head = pickedRowSql(inline);
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
				tarballSha256: String(row['tarball_sha256'] ?? ''),
				engineRange: String(row['engine_range'] ?? ''),
				manifestJson: String(row['manifest_json'] ?? '{}'),
				tarManifestJson: String(row['tar_manifest_json'] ?? '[]'),
				withdrawn: row['withdrawn'] !== null && row['withdrawn'] !== undefined
			});
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
): Promise<{ resolved: Record<string, string>; levels: ResolveNode[][]; chunks: string | null }> {
	const inline = options.inlineChunks === true;
	await getDdb(env);
	const raw = getClient(env);

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

		const memoHit: Array<[string, string[]]> = [];
		const missing: string[] = [];
		for (const full of frontier) {
			if (candidates.has(full)) continue;
			const hit = versionIndexGet(full);
			if (hit) memoHit.push([full, hit]);
			else missing.push(full);
		}
		if (missing.length > 0) {
			const fresh = await resolveCandidates(raw, missing);
			for (const [full, list] of fresh) {
				candidates.set(full, list);
				versionIndexSet(full, list);
			}
		}
		for (const [full, list] of memoHit) candidates.set(full, list);

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
		for (const full of frontier) {
			const versions = candidates.get(full) ?? [];
			const picked: string[] = [];
			for (const range of ranges.get(full) ?? []) {
				const pick = maxSatisfying(versions, range);
				if (!pick || picked.includes(pick)) continue;
				picked.push(pick);
				wanted.push({ full, version: pick });
			}
			if (picked.length === 0) blocked.add(full);
			picksByName.set(full, picked);
		}
		for (const [id, row] of await resolvePickedRows(raw, wanted, inline)) rows.set(id, row);

		const next: string[] = [];
		for (const full of frontier) {
			visited += 1;
			if (visited > RESOLVE_MAX_NODES) throw budgetError(`closure over ${RESOLVE_MAX_NODES} nodes`);
			for (const version of picksByName.get(full) ?? []) {
				const id = `${full}@${version}`;
				const row = rows.get(id);
				if (!row) continue;
				if (pruneHit(have, full, version, row.tarballSha256)) continue;
				for (const [name, range] of depsOf(id, row)) {
					if (addRange(name, range)) next.push(name);
				}
			}
		}
		frontier = next.sort();
	}

	const makeNode = (full: string, version: string, row: ResolveRow, deps: string[]): ResolveNode => ({
		full,
		version,
		path: `${full}@${version}/chunks`,
		integrity: row.tarballSha256,
		engineRange: row.engineRange,
		deps,
		yanked: row.status === 'yanked'
	});

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
		if (pruneHit(have, full, pick, row.tarballSha256)) {
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

	// One global switch, decided from the stored rows alone: over the cap the
	// whole response falls back to links and no node carries entries.
	let withEntries = inline;
	if (withEntries) {
		let total = 0;
		for (const node of nodes) total += rows.get(`${node.full}@${node.version}`)?.tarManifestJson.length ?? 0;
		if (total > RESOLVE_MAX_INLINE_BYTES) withEntries = false;
	}
	if (withEntries) {
		for (const node of nodes) {
			const row = rows.get(`${node.full}@${node.version}`);
			if (!row) continue;
			const entries = resolveEntries(row.tarManifestJson);
			if (entries) node.entries = entries;
		}
	}

	return {
		resolved,
		levels: order.map((ids) => ids.map((id) => byId.get(id)!)),
		chunks: options.advertiseChunks === true ? (withEntries ? 'inline' : 'links') : null
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
		generationBump(db)
	] as unknown as [BatchItem<'sqlite'>, ...BatchItem<'sqlite'>[]]);
	noteMutation();
}
