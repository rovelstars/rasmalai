import { describe, it, before, after, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import {
	POST as mutationPOST,
	GET as packageGET
} from '../src/routes/api/v0/packages/[...path]/+server.ts';
import { POST as publishPOST } from '../src/routes/api/v0/packages/+server.ts';
import { POST as resolvePOST } from '../src/routes/api/v0/resolve/+server.ts';
import { POST as gcPOST } from '../src/routes/api/admin/gc/+server.ts';
import { POST as downloadsPOST } from '../src/routes/api/admin/downloads/+server.ts';
import { DELETE as specialDELETE } from '../src/routes/api/scopes/[scope]/[target]/+server.ts';
import { GET as versionGET } from '../src/routes/api/version/+server.ts';
import { seedPackage } from './publish-helpers.js';
import {
	clearVersionIndex,
	dayString,
	getVersionIndexRow,
	listPackages,
	versionReuseBlocked
} from '../src/lib/server/db.js';

// Route conformance for the operator surface: the mutations the registry
// spec mandates, the self-host overrides, and the admin operations. Every
// test drives the real route handlers against a throwaway database; no
// production system is touched.

let fake: FakeLibsql;
const adminEnv = (): Record<string, string | undefined> => ({
	...fake.env,
	ADMIN_TOKEN: 'admin',
	ADMIN_DELETE_TOKEN: 'second'
});

function mutation(path: string, opts: { token?: string; body?: unknown; headers?: Record<string, string> } = {}): Promise<Response> {
	const request = new Request(`https://registry.test/api/v0/packages/${path}`, {
		method: 'POST',
		headers: {
			authorization: `Bearer ${opts.token ?? 'preview-token'}`,
			'content-type': 'application/json',
			...(opts.headers ?? {})
		},
		body: JSON.stringify(opts.body ?? {})
	});
	return mutationPOST({
		params: { path },
		request,
		url: new URL(`https://registry.test/api/v0/packages/${path}`),
		platform: { env: adminEnv() }
	} as never);
}

function read(path: string): Promise<Response> {
	return packageGET({
		params: { path },
		request: new Request(`https://registry.test/api/v0/packages/${path}`),
		url: new URL(`https://registry.test/api/v0/packages/${path}`),
		platform: { env: fake.env },
		setHeaders: () => {}
	} as never);
}

function resolve(requirements: Record<string, string>): Promise<Response> {
	const request = new Request('https://registry.test/api/v0/resolve', {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify({ requirements })
	});
	return resolvePOST({
		request,
		url: new URL('https://registry.test/api/v0/resolve'),
		platform: { env: fake.env }
	} as never);
}

function specialDelete(scope: string, target: string, admin: string, second: string): Promise<Response> {
	const request = new Request(`https://registry.test/api/scopes/${scope}/${target}`, {
		method: 'DELETE',
		headers: { authorization: `Bearer ${admin}`, 'x-rnx-second-approval': second }
	});
	return specialDELETE({
		params: { scope, target },
		request,
		url: new URL(`https://registry.test/api/scopes/${scope}/${target}`),
		platform: { env: adminEnv() }
	} as never);
}

function config(name: string, version: string): string {
	return `export default {\n    project: {\n        name: "${name}",\n        version: "${version}"\n    },\n    entries: { main: "src/main.rnx" }\n}\n`;
}

function publishFrame(name: string, version: string, requestId?: string): Record<string, unknown> {
	const frame: Record<string, unknown> = {
		name,
		version,
		files: [
			{ path: 'Project.config', content: config(name, version) },
			{ path: 'LICENSE', content: 'MIT License\n' },
			{ path: 'src/main.rnx', content: 'export fn main(): Void {\n  let x = 1;\n}\n' }
		],
		description: 'fixture',
		readme: `# ${name}\n`,
		license: 'MIT'
	};
	if (requestId !== undefined) frame['requestId'] = requestId;
	return frame;
}

function publish(name: string, version: string, frame: Record<string, unknown>, headers: Record<string, string> = {}): Promise<Response> {
	const request = new Request('https://registry.test/api/v0/packages', {
		method: 'POST',
		headers: {
			authorization: 'Bearer preview-token',
			'content-type': 'application/json',
			'x-rnx-protocol': '0',
			...headers
		},
		body: JSON.stringify(frame)
	});
	return publishPOST({ request, platform: { env: adminEnv() }, url: new URL('https://registry.test/api/v0/packages') } as never);
}

before(async () => {
	fake = await startFakeLibsql();
});

after(async () => {
	await fake.close();
});

beforeEach(() => {
	clearVersionIndex();
	fake.reset();
});

describe('discovery override', () => {
	it('answers the canonical host by default and the deployment host when configured', async () => {
		const plain = await versionGET({ platform: { env: fake.env }, setHeaders: () => {} } as never);
		assert.equal(plain.status, 200);
		assert.equal(((await plain.json()) as { registry: string }).registry, 'rasmalai.rovelstars.com');
		const custom = await versionGET(
			{ platform: { env: { ...fake.env, SITE_URL: 'https://registry.example.com/' } }, setHeaders: () => {} } as never
		);
		assert.equal(((await custom.json()) as { registry: string }).registry, 'registry.example.com');
		const junk = await versionGET(
			{ platform: { env: { ...fake.env, SITE_URL: 'not a host!!' } }, setHeaders: () => {} } as never
		);
		assert.equal(((await junk.json()) as { registry: string }).registry, 'rasmalai.rovelstars.com');
	});
});

describe('staff special delete', () => {
	it('refuses non-std scopes, missing approval, and a reused admin token', async () => {
		await seedPackage(fake.env, '@acme/gone', '1.0.0');
		const scoped = await specialDelete('acme', 'gone@1.0.0', 'admin', 'second');
		assert.equal(scoped.status, 403);
		await seedPackage(fake.env, '@std/fs', '1.0.0');
		const noSecond = await specialDelete('std', 'fs@1.0.0', 'admin', '');
		assert.equal(noSecond.status, 403);
		const reused = await specialDelete('std', 'fs@1.0.0', 'admin', 'admin');
		assert.equal(reused.status, 403);
		const row = await getVersionIndexRow(fake.env, '@std/fs', '1.0.0');
		assert.ok(row, 'a refused delete leaves the row alone');
	});

	it('hard-deletes, keeps the audit entry, and answers the 410 variant everywhere', async () => {
		await seedPackage(fake.env, '@std/net', '2.0.0');
		const res = await specialDelete('std', 'net@2.0.0', 'admin', 'second');
		assert.equal(res.status, 200);
		const meta = await read('@std/net@2.0.0');
		assert.equal(meta.status, 410);
		const metaBody = (await meta.json()) as Record<string, unknown>;
		assert.equal(metaBody['code'], 'withdrawn');
		assert.match(String(metaBody['message']), /std-purge/);
		const missing = await read('@std/net@9.9.9');
		assert.equal(missing.status, 404);
		clearVersionIndex();
		const resolved = await resolve({ '@std/net': '*' });
		assert.equal(resolved.status, 410);
		const resolvedBody = (await resolved.json()) as Record<string, unknown>;
		assert.match(String(resolvedBody['message']), /std-purge/);
		assert.equal(await versionReuseBlocked(fake.env, '@std/net', '2.0.0'), true);
		const audit = fake.db
			.prepare("SELECT count(*) AS c FROM audit_log WHERE action = 'special-delete' AND full_name = '@std/net'")
			.get() as { c: number };
		assert.equal(Number(audit.c), 1);
	});
});

describe('owner unpublish', () => {
	it('disappears the version inside the window and burns the number', async () => {
		await seedPackage(fake.env, '@acme/typo', '1.0.0');
		const res = await mutation('@acme/typo@1.0.0/unpublish');
		assert.equal(res.status, 200);
		const meta = await read('@acme/typo@1.0.0');
		assert.equal(meta.status, 404);
		clearVersionIndex();
		const resolved = await resolve({ '@acme/typo': '*' });
		assert.equal(resolved.status, 404);
		assert.equal(await versionReuseBlocked(fake.env, '@acme/typo', '1.0.0'), true);
	});

	it('replays the same idempotency key as a duplicate success', async () => {
		await seedPackage(fake.env, '@acme/retry', '1.0.0');
		const first = await mutation('@acme/retry@1.0.0/unpublish', { body: { requestId: 'req-dup-1' } });
		assert.equal(first.status, 200);
		const second = await mutation('@acme/retry@1.0.0/unpublish', { body: { requestId: 'req-dup-1' } });
		assert.equal(second.status, 200);
		assert.equal(((await second.json()) as { duplicate?: boolean }).duplicate, true);
	});

	it('refuses versions past the 72-hour window and taken-down versions', async () => {
		await seedPackage(fake.env, '@acme/old', '1.0.0');
		fake.db
			.prepare('UPDATE package_versions SET created_at = ? WHERE package_id = ?')
			.run(Math.floor(Date.now() / 1000) - 73 * 3600, 'pkg_@acme/old');
		const res = await mutation('@acme/old@1.0.0/unpublish');
		assert.equal(res.status, 409);
		assert.equal(((await res.json()) as { code: string }).code, 'unpublish-window-closed');
		await seedPackage(fake.env, '@acme/doomed', '1.0.0');
		const taken = await mutation('@acme/doomed@1.0.0/takedown', { token: 'admin', body: { reason: 'malware' } });
		assert.equal(taken.status, 200);
		const gone = await mutation('@acme/doomed@1.0.0/unpublish');
		assert.equal(gone.status, 410);
	});
});

describe('mutation idempotency', () => {
	it('replays a yank with the same key instead of conflicting', async () => {
		await seedPackage(fake.env, '@acme/yankme', '1.0.0');
		const first = await mutation('@acme/yankme@1.0.0/yank', { headers: { 'x-rnx-request-id': 'req-y-1' } });
		assert.equal(first.status, 200);
		const second = await mutation('@acme/yankme@1.0.0/yank', { headers: { 'x-rnx-request-id': 'req-y-1' } });
		assert.equal(second.status, 200);
		assert.equal(((await second.json()) as { duplicate?: boolean }).duplicate, true);
	});

	it('replays a takedown with the same key instead of answering 410', async () => {
		await seedPackage(fake.env, '@acme/takeme', '1.0.0');
		const first = await mutation('@acme/takeme@1.0.0/takedown', { token: 'admin', body: { reason: 'x', requestId: 'req-t-1' } });
		assert.equal(first.status, 200);
		const second = await mutation('@acme/takeme@1.0.0/takedown', { token: 'admin', body: { reason: 'x', requestId: 'req-t-1' } });
		assert.equal(second.status, 200);
		assert.equal(((await second.json()) as { duplicate?: boolean }).duplicate, true);
	});
});

describe('yank window', () => {
	const WINDOW_SECONDS = 72 * 3600;
	function backdate(full: string, createdAt: number): void {
		fake.db.prepare('UPDATE package_versions SET created_at = ? WHERE package_id = ?').run(createdAt, `pkg_${full}`);
	}
	function oldTimestamp(extraSeconds: number): number {
		return Math.floor(Date.now() / 1000) - WINDOW_SECONDS - extraSeconds;
	}

	it('lets the owner yank inside the window and serves the yanked status', async () => {
		await seedPackage(fake.env, '@acme/freshyank', '1.0.0');
		const res = await mutation('@acme/freshyank@1.0.0/yank');
		assert.equal(res.status, 200);
		assert.equal(((await res.json()) as { yankCause: unknown }).yankCause, null);
		const meta = await read('@acme/freshyank@1.0.0');
		assert.equal(meta.status, 200);
		const body = (await meta.json()) as Record<string, unknown>;
		assert.equal(body['status'], 'yanked');
		assert.equal(body['yankCause'], null);
	});

	it('refuses an owner yank one second past the window with the age, the window, and the admin path', async () => {
		await seedPackage(fake.env, '@acme/staleyank', '1.0.0');
		backdate('@acme/staleyank', oldTimestamp(1));
		const res = await mutation('@acme/staleyank@1.0.0/yank');
		assert.equal(res.status, 409);
		const body = (await res.json()) as { code: string; message: string };
		assert.equal(body.code, 'yank-window-closed');
		assert.match(body.message, /hours old/);
		assert.match(body.message, /72 hours/);
		assert.match(body.message, /admin/);
		assert.match(body.message, /vulnerability \| cve \| malicious \| legal/);
		const meta = await read('@acme/staleyank@1.0.0');
		assert.equal(((await meta.json()) as { status: string }).status, 'live');
	});

	it('rejects an admin yank without a recorded cause, at any age', async () => {
		await seedPackage(fake.env, '@acme/nocause-fresh', '1.0.0');
		const fresh = await mutation('@acme/nocause-fresh@1.0.0/yank', { token: 'admin', body: {} });
		assert.equal(fresh.status, 400);
		assert.equal(((await fresh.json()) as { code: string }).code, 'invalid-yank-cause');
		await seedPackage(fake.env, '@acme/nocause-old', '1.0.0');
		backdate('@acme/nocause-old', oldTimestamp(3600));
		const old = await mutation('@acme/nocause-old@1.0.0/yank', { token: 'admin', body: { reason: 'not a cause' } });
		assert.equal(old.status, 400);
		assert.match(((await old.json()) as { message: string }).message, /vulnerability \| cve \| malicious \| legal/);
	});

	it('records and serves every admin cause on late versions', async () => {
		const causes = ['vulnerability', 'cve', 'malicious', 'legal'];
		for (const [i, cause] of causes.entries()) {
			const name = `@acme/latecause${i}`;
			await seedPackage(fake.env, name, '1.0.0');
			backdate(name, oldTimestamp(3600));
			const res = await mutation(`${name}@1.0.0/yank`, { token: 'admin', body: { cause } });
			assert.equal(res.status, 200);
			assert.equal(((await res.json()) as { yankCause: string }).yankCause, cause);
			const meta = await read(`${name}@1.0.0`);
			assert.equal(meta.status, 200);
			const body = (await meta.json()) as Record<string, unknown>;
			assert.equal(body['status'], 'yanked');
			assert.equal(body['yankCause'], cause);
			const audit = fake.db
				.prepare("SELECT details_json AS d FROM audit_log WHERE action = 'yank' AND full_name = ? AND version = '1.0.0'")
				.get(name) as { d: string };
			assert.match(String(audit.d), new RegExp(`"cause":"${cause}"`));
			clearVersionIndex();
			const resolved = await resolve({ [name]: '1.0.0' });
			assert.equal(resolved.status, 200);
			const graph = (await resolved.json()) as { levels: Array<Array<Record<string, unknown>>> };
			const nodes = graph.levels.flat();
			assert.equal(
				nodes.some((n) => n['yanked'] === true),
				true
			);
			assert.equal(
				nodes.find((n) => n['full'] === name)?.['yankCause'],
				cause
			);
		}
	});

	it('keeps serving late-yanked metadata and pins to exact requirements', async () => {
		await seedPackage(fake.env, '@acme/pinnedlate', '1.0.0');
		backdate('@acme/pinnedlate', oldTimestamp(3600));
		const yanked = await mutation('@acme/pinnedlate@1.0.0/yank', { token: 'admin', body: { cause: 'CVE' } });
		assert.equal(yanked.status, 200);
		assert.equal(((await yanked.json()) as { yankCause: string }).yankCause, 'cve');
		const manifest = await read('@acme/pinnedlate@1.0.0/manifest');
		assert.equal(manifest.status, 200);
		clearVersionIndex();
		const resolved = await resolve({ '@acme/pinnedlate': '1.0.0' });
		assert.equal(resolved.status, 200);
	});

	it('keeps unpublish, burned numbers, and replay unchanged', async () => {
		await seedPackage(fake.env, '@acme/yankunpub', '1.0.0');
		const yanked = await mutation('@acme/yankunpub@1.0.0/yank');
		assert.equal(yanked.status, 200);
		const first = await mutation('@acme/yankunpub@1.0.0/unpublish', { body: { requestId: 'req-yu-1' } });
		assert.equal(first.status, 200);
		const second = await mutation('@acme/yankunpub@1.0.0/unpublish', { body: { requestId: 'req-yu-1' } });
		assert.equal(second.status, 200);
		assert.equal(((await second.json()) as { duplicate?: boolean }).duplicate, true);
		assert.equal(await versionReuseBlocked(fake.env, '@acme/yankunpub', '1.0.0'), true);
	});
});

describe('publish idempotency key transport', () => {
	it('stores the header key when the frame leaves it empty, and prefers the frame', async () => {
		const first = await publish('@acme/keyed', '1.0.0', publishFrame('@acme/keyed', '1.0.0'), {
			'x-rnx-request-id': 'req-header-1'
		});
		assert.equal(first.status, 201);
		const stored = await getVersionIndexRow(fake.env, '@acme/keyed', '1.0.0');
		assert.ok(stored);
		assert.equal(stored.row.requestId, 'req-header-1');
		const second = await publish('@acme/keyed', '1.0.1', publishFrame('@acme/keyed', '1.0.1', 'req-frame-2'), {
			'x-rnx-request-id': 'req-header-ignored'
		});
		assert.equal(second.status, 201);
		const stored2 = await getVersionIndexRow(fake.env, '@acme/keyed', '1.0.1');
		assert.ok(stored2);
		assert.equal(stored2.row.requestId, 'req-frame-2');
	});
});

describe('admin garbage collection', () => {
	it('dry-runs without deleting, then sweeps and prunes', async () => {
		const old = Math.floor(Date.now() / 1000) - 31 * 86400;
		fake.db
			.prepare('INSERT INTO chunks (hash, size_bytes, bytes, first_seen_at) VALUES (?, ?, ?, ?)')
			.run('a'.repeat(64), 3, Buffer.from('abc'), old);
		const dryRequest = new Request('https://registry.test/api/admin/gc?dry_run=1', {
			method: 'POST',
			headers: { authorization: 'Bearer admin', 'content-type': 'application/json' },
			body: '{}'
		});
		const dry = await gcPOST({ request: dryRequest, platform: { env: adminEnv() }, url: new URL(dryRequest.url) } as never);
		assert.equal(dry.status, 200);
		const dryBody = (await dry.json()) as Record<string, unknown>;
		assert.equal(dryBody['dryRun'], true);
		assert.deepEqual(dryBody['wouldDelete'], ['a'.repeat(64)]);
		const kept = fake.db.prepare("SELECT count(*) AS c FROM chunks WHERE hash = ?").get('a'.repeat(64)) as { c: number };
		assert.equal(Number(kept.c), 1);
		const ancient = dayString(Math.floor(Date.now() / 1000) - 731 * 86400);
		await seedPackage(fake.env, '@acme/prunee', '1.0.0');
		fake.db.prepare("INSERT INTO download_daily (package_id, day, downloads) VALUES ('pkg_@acme/prunee', ?, 5)").run(ancient);
		fake.db
			.prepare("INSERT INTO audit_log (action, full_name, version, details_json, created_at) VALUES ('purge-skipped', '', '', '{}', ?)")
			.run(Math.floor(Date.now() / 1000) - 366 * 86400);
		const before = fake.db
			.prepare("SELECT count(*) AS c FROM audit_log WHERE action = 'publish'")
			.get() as { c: number };
		const runRequest = new Request('https://registry.test/api/admin/gc', {
			method: 'POST',
			headers: { authorization: 'Bearer admin', 'content-type': 'application/json' },
			body: '{}'
		});
		const run = await gcPOST({ request: runRequest, platform: { env: adminEnv() }, url: new URL(runRequest.url) } as never);
		assert.equal(run.status, 200);
		const runBody = (await run.json()) as Record<string, unknown>;
		assert.equal(runBody['deleted'], 1);
		assert.deepEqual(runBody['pruned'], { downloads: 1, audit: 1 });
		const after = fake.db
			.prepare("SELECT count(*) AS c FROM audit_log WHERE action = 'publish'")
			.get() as { c: number };
		assert.equal(Number(after.c), Number(before.c), 'version history rows are never pruned');
	});
});

describe('admin download rollup', () => {
	it('records one batched daily row per package and the catalog reads it', async () => {
		await seedPackage(fake.env, '@acme/counted', '1.0.0');
		const day = dayString(Math.floor(Date.now() / 1000));
		const request = new Request('https://registry.test/api/admin/downloads', {
			method: 'POST',
			headers: { authorization: 'Bearer admin', 'content-type': 'application/json' },
			body: JSON.stringify({ day, counts: { '@acme/counted': 41 } })
		});
		const res = await downloadsPOST({ request, platform: { env: adminEnv() } } as never);
		assert.equal(res.status, 200);
		assert.equal(((await res.json()) as { recorded: number }).recorded, 1);
		const catalog = await listPackages(fake.env);
		const row = catalog.find((p) => p.name === '@acme/counted');
		assert.ok(row);
		assert.equal(row.downloads, 41);
		const bad = new Request('https://registry.test/api/admin/downloads', {
			method: 'POST',
			headers: { authorization: 'Bearer admin', 'content-type': 'application/json' },
			body: JSON.stringify({ day: 'yesterday', counts: {} })
		});
		const badRes = await downloadsPOST({ request: bad, platform: { env: adminEnv() } } as never);
		assert.equal(badRes.status, 400);
	});
});
