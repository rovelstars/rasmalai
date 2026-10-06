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
	downloads: integer('downloads').notNull().default(0),
	stars: integer('stars').notNull().default(0),
	tags: text('tags').notNull(),
	keywords: text('keywords').default(''),
	createdAt: integer('created_at').notNull(),
	updatedAt: integer('updated_at').notNull()
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

export const manifestChunks = sqliteTable(
	'manifest_chunks',
	{
		chunkHash: text('chunk_hash').notNull(),
		versionId: text('version_id').notNull(),
		ord: integer('ord').notNull()
	},
	(t) => [primaryKey({ columns: [t.versionId, t.ord] })]
);

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

export const users = sqliteTable('users', {
	id: integer('id').primaryKey(),
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
