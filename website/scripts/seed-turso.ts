import { readFileSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createClient } from '@libsql/client/web';
import { BASE_SCHEMA } from '../src/lib/server/db.js';

function loadDotEnv(root: string): void {
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

async function main(): Promise<void> {
	const root = join(dirname(fileURLToPath(import.meta.url)), '..');
	loadDotEnv(root);
	const url = process.env['TURSO_DATABASE_URL'];
	const token = process.env['TURSO_AUTH_TOKEN'];
	if (!url) {
		console.error(
			'seed-turso: missing TURSO_DATABASE_URL (and TURSO_AUTH_TOKEN). ' +
				'Set them in the environment or website/.env, then retry.'
		);
		process.exit(1);
	}
	const db = createClient({ url, authToken: token });
	for (const stmt of BASE_SCHEMA.split(';')) {
		const sql = stmt.trim();
		if (sql) await db.execute(sql);
	}
	const now = Math.floor(Date.now() / 1000);
	await db.execute({
		sql: `INSERT INTO scopes (name, owner, reserved, created_at) VALUES (?, ?, ?, ?)
		      ON CONFLICT(name) DO NOTHING`,
		args: ['std', 'rovelstars', 1, now]
	});
	const pkgs = await db.execute('SELECT COUNT(*) AS n FROM packages');
	const vers = await db.execute('SELECT COUNT(*) AS n FROM package_versions');
	console.log(
		`seed-turso: schema ready, std scope reserved, ${pkgs.rows[0].n} package(s), ${vers.rows[0].n} version(s)`
	);
}

main().catch((e) => {
	console.error('seed-turso: ' + (e instanceof Error ? e.message : String(e)));
	process.exit(1);
});
