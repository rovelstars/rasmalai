// Seeds one registry package per stdlib module (@std/<name>) at the
// current toolchain version. Reads website/.env for Turso credentials.
// Run after releases and toolchain bumps:
//
//   node scripts/seed-std.mjs
//
// Each module row carries the full doc JSON snapshot for that version,
// so docs stay immutable and cacheable per (package, version).
import { readFileSync, existsSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createClient } from '@libsql/client/web';

function loadDotEnv(root) {
	const envFile = join(root, '.env');
	if (!existsSync(envFile)) return;
	for (const line of readFileSync(envFile, 'utf8').split('\n')) {
		const trimmed = line.trim();
		if (!trimmed || trimmed.startsWith('#')) continue;
		const eq = trimmed.indexOf('=');
		if (eq < 0) continue;
		const key = trimmed.slice(0, eq).trim();
		const value = trimmed
			.slice(eq + 1)
			.trim()
			.replace(/^"(.*)"$/, '$1')
			.replace(/^'(.*)'$/, '$1');
		if (!(key in process.env)) process.env[key] = value;
	}
}

async function main() {
	const root = join(dirname(fileURLToPath(import.meta.url)), '..');
	const repo = join(root, '..');
	loadDotEnv(root);
	const url = process.env['TURSO_DATABASE_URL'];
	const token = process.env['TURSO_AUTH_TOKEN'];
	if (!url) {
		console.error('seed-std: missing TURSO_DATABASE_URL in environment or website/.env');
		process.exit(1);
	}
	const version = readFileSync(join(repo, 'compiler', 'cli', 'Cargo.toml'), 'utf8')
		.match(/^version = "(.*)"$/m)?.[1];
	if (!version) {
		console.error('seed-std: cannot read cli version');
		process.exit(1);
	}
	const outDir = join(root, '.std-doc');
	mkdirSync(outDir, { recursive: true });
	const rnxBin = process.env['RNX_BIN'];
	if (rnxBin) {
		execFileSync(rnxBin, ['doc', '--json', '--stdlib', '--out-dir', outDir], { stdio: 'inherit' });
	} else {
		execFileSync('cargo', ['run', '-q', '-p', 'cli', '--', 'doc', '--json', '--stdlib', '--out-dir', outDir], {
			cwd: repo,
			stdio: 'inherit'
		});
	}
	const api = JSON.parse(readFileSync(join(outDir, 'api.json'), 'utf8'));
	const modules = api.modules ?? [];
	if (modules.length === 0) {
		console.error('seed-std: api.json has no modules');
		process.exit(1);
	}
	const db = createClient({ url, authToken: token });
	const now = Math.floor(Date.now() / 1000);
	let created = 0;
	for (const m of modules) {
		const name = `@std/${m.name}`;
		const id = `pkg_@${'std'}/${m.name}`;
		const docJson = JSON.stringify({ modules: [m] });
		const checksum = createHash('sha256').update(docJson).digest('hex');
		await db.execute({
			sql: `INSERT INTO scopes (name, owner, reserved, created_at) VALUES (?, ?, ?, ?)
			      ON CONFLICT(name) DO NOTHING`,
			args: ['std', 'rovelstars', 1, now]
		});
		await db.execute({
			sql: `INSERT INTO packages (id, scope, name, description, author, repository, license, downloads, stars, tags, created_at, updated_at)
			      VALUES (?, ?, ?, ?, ?, ?, ?, 0, 0, ?, ?, ?)
			      ON CONFLICT(scope, name) DO UPDATE SET description = excluded.description, updated_at = excluded.updated_at`,
			args: [
				id,
				'std',
				m.name,
				(m.docs?.description ?? `${name} standard library module`).split('\n')[0],
				'Rovel Stars',
				'https://github.com/rovelstars/rasmalai',
				'MIT',
				'stdlib',
				now,
				now
			]
		});
		const res = await db.execute({
			sql: `INSERT INTO package_versions (id, package_id, version, readme_markdown, doc_json, checksum, created_at)
			      VALUES (?, ?, ?, ?, ?, ?, ?)
			      ON CONFLICT(package_id, version) DO NOTHING`,
			args: [
				`ver_${id}_${version}`,
				id,
				version,
				`# ${name}\n\nStandard library module, version ${version}.\n`,
				docJson,
				checksum,
				now
			]
		});
		if ((res.rowsAffected ?? 0) > 0) created++;
	}
	console.log(`seed-std: ${modules.length} std module(s) at v${version}, ${created} new version(s)`);
}

main().catch((e) => {
	console.error('seed-std: ' + (e instanceof Error ? e.message : String(e)));
	process.exit(1);
});
