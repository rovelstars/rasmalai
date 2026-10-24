import { sqliteTable, text, integer, blob, primaryKey } from 'drizzle-orm/sqlite-core';

export const scopes = sqliteTable('scopes', {
	name: text('name').primaryKey(),
	owner: text('owner').notNull(),
	reserved: integer('reserved').notNull().default(0),
	createdAt: integer('created_at').notNull()
});

export const packages = sqliteTable('packages', {
	id: text('id').primaryKey(),
	scope: text('scope').notNull().default(''),
	name: text('name').notNull(),
	owner: text('owner').notNull().default(''),
	description: text('description').notNull(),
	author: text('author').notNull(),
	repository: text('repository'),
	license: text('license').notNull().default('MIT'),
	licenseId: integer('license_id'),
	downloads: integer('downloads').notNull().default(0),
	stars: integer('stars').notNull().default(0),
	tags: text('tags').notNull(),
	keywords: text('keywords').default(''),
	createdAt: integer('created_at').notNull(),
	updatedAt: integer('updated_at').notNull(),
	// Monotonic per-package counter of the mutations that change the
	// candidate set of this package. The resolve memo carries the value it
	// was built with and revalidates it against the row it reads.
	mutationCount: integer('mutation_count').notNull().default(0)
});

export const packageVersions = sqliteTable('package_versions', {
	id: text('id').primaryKey(),
	packageId: text('package_id').notNull(),
	version: text('version').notNull(),
	readmeMarkdown: text('readme_markdown').notNull(),
	docJson: text('doc_json').notNull(),
	checksum: text('checksum').notNull(),
	status: text('status').notNull().default('live'),
	createdAt: integer('created_at').notNull(),
	semverMajor: integer('semver_major').notNull().default(0),
	semverMinor: integer('semver_minor').notNull().default(0),
	semverPatch: integer('semver_patch').notNull().default(0),
	prerelease: text('prerelease').notNull().default(''),
	engineRange: text('engine_range').notNull().default(''),
	manifestJson: text('manifest_json').notNull().default('{}'),
	guidesJson: text('guides_json').notNull().default('[]'),
	tarballSha256: text('tarball_sha256').notNull().default(''),
	tarManifestJson: text('tar_manifest_json').notNull().default('[]'),
	manifestDigest: text('manifest_digest').notNull().default(''),
	verdictKey: text('verdict_key').notNull().default(''),
	requestId: text('request_id').notNull().default('')
});

export const benchmarkRuns = sqliteTable('benchmark_runs', {
	id: integer('id').primaryKey(),
	githubRunId: integer('github_run_id').notNull(),
	commitSha: text('commit_sha').notNull(),
	snapshotJson: text('snapshot_json').notNull(),
	createdAt: integer('created_at').notNull()
});

export const tombstones = sqliteTable(
	'tombstones',
	{
		packageId: text('package_id').notNull(),
		version: text('version').notNull(),
		reason: text('reason').notNull().default(''),
		createdAt: integer('created_at').notNull()
	},
	(t) => [primaryKey({ columns: [t.packageId, t.version] })]
);

export const auditLog = sqliteTable('audit_log', {
	id: integer('id').primaryKey(),
	action: text('action').notNull(),
	fullName: text('full_name').notNull(),
	version: text('version').notNull().default(''),
	detailsJson: text('details_json').notNull().default('{}'),
	requestId: text('request_id').notNull().default(''),
	createdAt: integer('created_at').notNull()
});

export const chunks = sqliteTable('chunks', {
	hash: text('hash').primaryKey(),
	sizeBytes: integer('size_bytes').notNull(),
	bytes: blob('bytes', { mode: 'buffer' }).notNull(),
	firstSeenAt: integer('first_seen_at').notNull()
});

export const packageDeps = sqliteTable(
	'package_deps',
	{
		packageId: text('package_id').notNull(),
		depName: text('dep_name').notNull()
	},
	(t) => [primaryKey({ columns: [t.packageId, t.depName] })]
);

export const downloadDaily = sqliteTable(
	'download_daily',
	{
		packageId: text('package_id').notNull(),
		day: text('day').notNull(),
		downloads: integer('downloads').notNull().default(0)
	},
	(t) => [primaryKey({ columns: [t.packageId, t.day] })]
);

export const users = sqliteTable('users', {	id: integer('id').primaryKey(),
	username: text('username').notNull().unique(),
	passwordHash: text('password_hash').notNull(),
	passwordSalt: text('password_salt').notNull(),
	passwordParams: text('password_params').notNull(),
	isAdmin: integer('is_admin').notNull().default(0),
	disclaimerAck: integer('disclaimer_ack').notNull().default(0),
	createdAt: integer('created_at').notNull()
});

export const sessions = sqliteTable('sessions', {
	tokenHash: text('token_hash').primaryKey(),
	userId: integer('user_id').notNull(),
	createdAt: integer('created_at').notNull(),
	expiresAt: integer('expires_at').notNull(),
	revokedAt: integer('revoked_at').notNull().default(0)
});

export const orgs = sqliteTable('orgs', {
	id: integer('id').primaryKey(),
	scope: text('scope').notNull().unique(),
	ownerUserId: integer('owner_user_id').notNull(),
	createdAt: integer('created_at').notNull()
});

export const authAttempts = sqliteTable(
	'auth_attempts',
	{
		ip: text('ip').notNull(),
		username: text('username').notNull(),
		attempts: integer('attempts').notNull().default(0),
		windowStart: integer('window_start').notNull()
	},
	(t) => [primaryKey({ columns: [t.ip, t.username] })]
);

export const capabilityDomains = sqliteTable('capability_domains', {
	id: integer('id').primaryKey(),
	name: text('name').notNull().unique()
});

export const versionCapabilities = sqliteTable(
	'version_capabilities',
	{
		versionId: text('version_id').notNull(),
		domainId: integer('domain_id').notNull(),
		arg: text('arg').notNull().default(''),
		reason: text('reason')
	},
	(t) => [primaryKey({ columns: [t.versionId, t.domainId, t.arg] })]
);

export const licenses = sqliteTable('licenses', {
	id: integer('id').primaryKey(),
	spdx: text('spdx').notNull().unique()
});

// Per-file analysis facts, keyed by the canonical content id the server
// itself derived. Content-addressed: a reindent produces the same
// canonical bytes, the same hash, and the same facts. GC'd with the
// chunks (a fact whose canonical bytes are gone is unreachable).
export const canonicalFacts = sqliteTable('canonical_facts', {
	canonicalHash: text('canonical_hash').primaryKey(),
	chunkList: text('chunk_list').notNull().default('[]'),
	importList: text('import_list').notNull().default('[]'),
	evidence: text('evidence').notNull().default('[]'),
	firstSeenAt: integer('first_seen_at').notNull()
});

// One row per package: the resolved import edge list, the declared
// permission digest and the per-file canonical hashes of the latest live
// version. Replaced on every publish; the diff against it decides whether
// a republish can skip re-analysis.
export const packageEdges = sqliteTable('package_edges', {
	packageId: text('package_id').primaryKey(),
	edgeList: text('edge_list').notNull().default('[]'),
	permsDigest: text('perms_digest').notNull().default(''),
	fileList: text('file_list').notNull().default('[]'),
	updatedAt: integer('updated_at').notNull()
});
