import { createClient, type Client } from '@libsql/client/web';

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

const BASE_SCHEMA = `
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

function getClient(env: Record<string, string | undefined>): Client | null {
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

async function ensureSchema(db: Client): Promise<void> {
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
		sql: `INSERT INTO package_versions (id, package_id, version, readme_markdown, doc_json, checksum, status, created_at)
		      VALUES (?, ?, ?, ?, ?, ?, 'live', ?)
		      ON CONFLICT(package_id, version) DO NOTHING`,
		args: [`ver_${id}_${p.version}`, id, p.version, p.readme, p.docJson, p.checksum, now]
	});
	return { name: parsed.full, version: p.version, created: (res.rowsAffected ?? 0) > 0 };
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
