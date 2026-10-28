import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { listPackages, publishCanonicalVersion, yankVersion } from '../src/lib/server/db.js';
import { chunkCanonicalFiles, manifestDigest } from '../src/lib/server/chunks.js';
import {
	registriesAdvisoryFor,
	registriesKeysIn,
	registriesRejection
} from '../src/lib/server/publishRules.js';
import { listActiveAdvisories } from '../src/lib/server/advisories.js';
import { analyzeTree } from '../src/lib/server/analysis.js';
import { seedPayload } from './publish-helpers.js';
import { POST as publishPOST } from '../src/routes/api/v0/packages/+server.ts';
import { GET as rulesGET, POST as rulesFilePOST } from '../src/routes/api/admin/publish-rules/+server.ts';

let fake: FakeLibsql;

const MAIN = 'fn Main(): Int {\n    return 0;\n}\n';
const REGISTRY_EXTRA = ',\n    registry: { url: "https://rnx.example.invalid" }';
const REGISTRIES_EXTRA = ',\n    registries: { "@acme": { url: "https://acme.example.invalid" } }';
const BOTH_EXTRA = `${REGISTRY_EXTRA}${REGISTRIES_EXTRA}`;
const SHAPES = [
	{ name: 'agree-registry', extra: REGISTRY_EXTRA, keys: ['registry'] },
	{ name: 'agree-registries', extra: REGISTRIES_EXTRA, keys: ['registries'] },
	{ name: 'agree-both', extra: BOTH_EXTRA, keys: ['registries', 'registry'] }
];

function configText(name: string, version: string, extraConfig = ''): string {
	return `export default {\n    project: {\n        name: "${name}",\n        version: "${version}"\n    },\n    entries: { main: "src/main.rnx" }${extraConfig}\n}\n`;
}

function frame(
	name: string,
	version: string,
	main: string,
	extraConfig = ''
): Record<string, unknown> {
	return {
		name,
		version,
		files: [
			{ path: 'Project.config', content: configText(name, version, extraConfig) },
			{ path: 'LICENSE', content: 'MIT License\n' },
			{ path: 'src/main.rnx', content: main }
		],
		description: 'fixture',
		readme: `# ${name}\n`,
		license: 'MIT'
	};
}

async function publish(
	env: Record<string, string | undefined>,
	name: string,
	version: string,
	main: string,
	extraConfig = ''
) {
	const request = new Request('https://registry.test/api/v0/packages', {
		method: 'POST',
		headers: {
			'content-type': 'application/json',
			'X-RNX-Protocol': '0',
			authorization: 'Bearer preview-token-123'
		},
		body: JSON.stringify(frame(name, version, main, extraConfig))
	});
	return publishPOST({
		request,
		platform: { env },
		url: new URL('https://registry.test/api/v0/packages')
	} as never);
}

// A stored package the publish route could never mint: the frame bypasses
// the gate by writing chunks and the version row directly, the way a
// pre-flip violator sits in the store.
async function seedStored(
	env: Record<string, string | undefined>,
	name: string,
	version: string,
	extraConfig = ''
): Promise<void> {
	const enc = new TextEncoder();
	const chunked = await chunkCanonicalFiles([
		{ name: 'Project.config', bytes: enc.encode(configText(name, version, extraConfig)) },
		{ name: 'src/main.rnx', bytes: enc.encode(MAIN) }
	]);
	const digest = await manifestDigest(chunked.entries);
	const res = await publishCanonicalVersion(
		env,
		seedPayload(name, version, {
			checksum: digest,
			manifestDigest: digest,
			entries: chunked.entries,
			units: chunked.units
		})
	);
	assert.ok(res.created, `${name}@${version} seeds into the store`);
}

function rulesTarget(limit?: number): string {
	return limit === undefined
		? 'https://registry.test/api/admin/publish-rules'
		: `https://registry.test/api/admin/publish-rules?limit=${limit}`;
}

function scan(env: Record<string, string | undefined>, limit?: number): Promise<Response> {
	const target = rulesTarget(limit);
	return rulesGET({
		request: new Request(target, { headers: { authorization: 'Bearer admin' } }),
		platform: { env: { ...env, ADMIN_TOKEN: 'admin' } },
		url: new URL(target)
	} as never);
}

function fileAdvisories(env: Record<string, string | undefined>, limit?: number): Promise<Response> {
	const target = rulesTarget(limit);
	const request = new Request(target, {
		method: 'POST',
		headers: { authorization: 'Bearer admin' }
	});
	return rulesFilePOST({
		request,
		platform: { env: { ...env, ADMIN_TOKEN: 'admin' } },
		url: new URL(target)
	} as never);
}

before(async () => {
	fake = await startFakeLibsql();
});

after(async () => {
	await fake.close();
});

describe('registries publish gate (rejection)', () => {
	it('rejects the registry key alone, naming the key and the fix', async () => {
		const res = await publish(fake.env, 'gate-registry', '1.0.0', MAIN, REGISTRY_EXTRA);
		assert.equal(res.status, 400);
		const body = (await res.json()) as { error?: string };
		assert.match(body.error ?? '', /E108/);
		assert.match(body.error ?? '', /`registry`/);
		assert.match(body.error ?? '', /delete the `registry` and `registries` keys/);
		assert.match(body.error ?? '', /next version/);
	});

	it('rejects the registries key alone', async () => {
		const res = await publish(fake.env, 'gate-registries', '1.0.0', MAIN, REGISTRIES_EXTRA);
		assert.equal(res.status, 400);
		const body = (await res.json()) as { error?: string };
		assert.match(body.error ?? '', /E108/);
		assert.match(body.error ?? '', /`registries`/);
		assert.match(body.error ?? '', /delete the `registry` and `registries` keys/);
	});

	it('rejects both keys together, naming each', async () => {
		const res = await publish(fake.env, 'gate-both', '1.0.0', MAIN, BOTH_EXTRA);
		assert.equal(res.status, 400);
		const body = (await res.json()) as { error?: string };
		assert.match(body.error ?? '', /`registry`/);
		assert.match(body.error ?? '', /`registries`/);
	});

	it('stores nothing on rejection, so the clean retry keeps its number', async () => {
		const rejected = await publish(fake.env, 'gate-retry', '1.0.0', MAIN, REGISTRIES_EXTRA);
		assert.equal(rejected.status, 400);
		const retry = await publish(fake.env, 'gate-retry', '1.0.0', MAIN);
		assert.equal(retry.status, 201);
		const body = (await retry.json()) as {
			reportOnly?: { registries?: { keys?: string[] } };
		};
		assert.deepEqual(body.reportOnly?.registries?.keys, []);
	});

	it('still publishes a clean package with an empty verdict', async () => {
		const res = await publish(fake.env, 'gate-clean', '1.0.0', MAIN);
		assert.equal(res.status, 201);
		const body = (await res.json()) as {
			reportOnly?: { registries?: { keys?: string[] } };
		};
		assert.deepEqual(body.reportOnly?.registries?.keys, []);
		assert.ok(!('s103' in (body.reportOnly ?? {})), JSON.stringify(body.reportOnly));
	});
});

describe('admin publish-rules scan bounds', () => {
	it('refuses the scan without the admin token', async () => {
		const target = rulesTarget();
		const res = await rulesGET({
			request: new Request(target),
			platform: { env: { ...fake.env, ADMIN_TOKEN: 'admin' } },
			url: new URL(target)
		} as never);
		assert.equal(res.status, 401);
		const denied = await rulesFilePOST({
			request: new Request(target, { method: 'POST' }),
			platform: { env: { ...fake.env } },
			url: new URL(target)
		} as never);
		assert.equal(denied.status, 401);
	});

	it('bounds the scan with limit and reports truncation', async () => {
		const res = await scan(fake.env, 1);
		assert.equal(res.status, 200);
		const body = (await res.json()) as { scanned: number; truncated: boolean; limit: number };
		assert.equal(body.limit, 1);
		assert.ok(body.scanned <= 1);
		assert.equal(body.truncated, true);
	});

	it('completes a large mixed scan in a bounded number of store round trips', async () => {
		const env = fake.env;
		const depFrame = {
			name: '@std/os',
			version: '0.1.0',
			files: [
				{
					path: 'Project.config',
					content:
						'export default {\n    project: {\n        name: "@std/os",\n        version: "0.1.0"\n    },\n    entries: { main: "src/lib.rnx" }\n}\n'
				},
				{ path: 'LICENSE', content: 'MIT License\n' },
				{ path: 'src/lib.rnx', content: 'export fn ident(x: Int): Int {\n  return x;\n}\n' }
			],
			description: 'fixture',
			readme: '# oslib\n',
			license: 'MIT'
		};
		const depRequest = new Request('https://registry.test/api/v0/packages', {
			method: 'POST',
			headers: {
				'content-type': 'application/json',
				'X-RNX-Protocol': '0',
				authorization: 'Bearer preview-token-123'
			},
			body: JSON.stringify(depFrame)
		});
		const depRes = await publishPOST({
			request: depRequest,
			platform: { env },
			url: new URL('https://registry.test/api/v0/packages')
		} as never);
		assert.equal(depRes.status, 201);
		const depMain = 'import { ident } from "@std/os";\nfn Main(): Int {\n    return ident(0);\n}\n';
		for (let i = 0; i < 4; i++) {
			const res = await publish(env, `scanbatch-dep-${i}`, '1.0.0', depMain);
			assert.equal(res.status, 201);
		}
		for (let i = 0; i < 20; i++) {
			const name = `scanbatch-clean-${String(i).padStart(2, '0')}`;
			const res = await publish(env, name, '1.0.0', MAIN);
			assert.equal(res.status, 201);
		}
		for (let i = 0; i < 6; i++) {
			await seedStored(env, `scanbatch-carrier-${i}`, '1.0.0', REGISTRIES_EXTRA);
		}
		const yankClean = await yankVersion(env, 'scanbatch-clean-00', '1.0.0', 'req-scan-yank');
		assert.ok(yankClean.ok);
		fake.reset();
		const res = await scan(env);
		assert.equal(res.status, 200);
		const body = (await res.json()) as {
			scanned: number;
			skipped: Array<{ name: string; version: string | null; reason: string }>;
			truncated: boolean;
			registries: { count: number; packages: Array<{ name: string; version: string; keys: string[] }> };
		};
		assert.equal(body.truncated, false);
		const total = (await listPackages(env)).length;
		assert.equal(body.scanned + body.skipped.length, total);
		for (const s of body.skipped) {
			assert.ok(!s.reason.includes('Failed query'), `${s.name}: ${s.reason}`);
		}
		assert.ok(fake.counters.roundTrips <= 16, `store round trips: ${fake.counters.roundTrips}`);
		assert.equal(body.registries.count, 6);
		assert.deepEqual(
			body.skipped.map((s) => s.name),
			['scanbatch-clean-00']
		);
		assert.equal(body.skipped[0].reason, 'no-live-version');
	});

	it('memoizes missing @std dep loads within one analysis cache', async () => {
		const files = {
			'src/main.rnx': 'import { nope } from "@std/nosuchmod";\nfn Main(): Int {\n    return 0;\n}\n'
		};
		const entries = { main: 'src/main.rnx', bins: {} as Record<string, string> };
		const cache = new Map();
		fake.reset();
		await assert.rejects(analyzeTree(fake.env, files, entries, cache), /is not published/);
		const firstTrips = fake.counters.roundTrips;
		assert.ok(firstTrips > 0, 'the first load reads the store');
		await assert.rejects(analyzeTree(fake.env, files, entries, cache), /is not published/);
		assert.equal(fake.counters.roundTrips, firstTrips);
	});
});

describe('scan and gate agree on every shape', () => {
	it('the gate rejects exactly the shapes the scan counts', async () => {
		const env = fake.env;
		for (const shape of SHAPES) {
			await seedStored(env, shape.name, '1.0.0', shape.extra);
			const res = await publish(env, `${shape.name}-live`, '1.0.0', MAIN, shape.extra);
			assert.equal(res.status, 400, shape.name);
		}
		await seedStored(env, 'agree-clean', '1.0.0');
		const clean = await publish(env, 'agree-clean-live', '1.0.0', MAIN);
		assert.equal(clean.status, 201);
		const res = await scan(env);
		assert.equal(res.status, 200);
		const body = (await res.json()) as {
			scanned: number;
			skipped: unknown[];
			truncated: boolean;
			registries: { count: number; packages: Array<{ name: string; version: string; keys: string[] }> };
		};
		const counted = new Map(body.registries.packages.map((p) => [p.name, p.keys]));
		for (const shape of SHAPES) {
			assert.deepEqual(counted.get(shape.name), shape.keys, shape.name);
		}
		assert.ok(!counted.has('agree-clean'), 'clean package is not counted');
		assert.ok(!counted.has('agree-clean-live'), 'clean package is not counted');
	});

	it('the shared predicate drives both verdicts', async () => {
		const env = fake.env;
		const fixtures = [...SHAPES, { name: 'agree-clean-shape', extra: '', keys: [] as string[] }];
		for (const shape of fixtures) {
			const analysis = await analyzeTree(
				env,
				{ 'src/main.rnx': MAIN },
				{ main: 'src/main.rnx', bins: {}, configText: configText(shape.name, '1.0.0', shape.extra) }
			);
			const keys = registriesKeysIn(analysis.reportOnly);
			assert.deepEqual(keys, shape.keys, shape.name);
			assert.equal(registriesRejection(keys) !== null, keys.length > 0, shape.name);
		}
	});

	it('the advisory shape names the cause and the remedy', () => {
		const input = registriesAdvisoryFor('agree-registries', '1.0.0', ['registries']);
		assert.equal(input.severity, 'critical');
		assert.equal(input.package, 'agree-registries');
		assert.equal(input.affected, '1.0.0');
		assert.match(input.detail ?? '', /registries/);
		assert.match(input.detail ?? '', /delete the `registry` and `registries` keys/);
		assert.match(input.detail ?? '', /next version/);
	});
});

describe('stored-registries advisories', () => {
	it('files a critical advisory for every stored violator', async () => {
		const env = fake.env;
		const before = await scan(env);
		const counted = ((await before.json()) as { registries: { count: number } }).registries.count;
		assert.ok(counted > 0, 'earlier tests seeded violators');
		const res = await fileAdvisories(env);
		assert.equal(res.status, 200);
		const outcome = (await res.json()) as { count: number; filed: string[]; present: string[] };
		assert.equal(outcome.count, counted);
		assert.equal(outcome.filed.length, counted);
		assert.deepEqual(outcome.present, []);
		const active = await listActiveAdvisories(env);
		for (const id of outcome.filed) {
			const advisory = active.find((a) => a.id === id);
			assert.ok(advisory, `${id} is active`);
			assert.equal(advisory.severity, 'critical');
			assert.match(advisory.detail, /delete the `registry` and `registries` keys/);
			assert.match(advisory.detail, /next version/);
		}
		const probe = active.find((a) => a.package === 'agree-registries');
		assert.ok(probe, 'a violating package carries the advisory');
		assert.equal(probe.severity, 'critical');
		assert.equal(probe.affected, '1.0.0');
		assert.match(probe.detail, /registries/);
	});

	it('converges: a second run files nothing new', async () => {
		const res = await fileAdvisories(fake.env);
		assert.equal(res.status, 200);
		const outcome = (await res.json()) as { count: number; filed: string[]; present: string[] };
		assert.ok(outcome.count > 0);
		assert.deepEqual(outcome.filed, []);
		assert.equal(outcome.present.length, outcome.count);
	});

	it('covers a newly seeded violator with cause and remedy', async () => {
		const env = fake.env;
		await seedStored(env, 'adv-carrier', '2.0.0', REGISTRIES_EXTRA);
		const expected = registriesAdvisoryFor('adv-carrier', '2.0.0', ['registries']).id;
		const res = await fileAdvisories(env);
		assert.equal(res.status, 200);
		const outcome = (await res.json()) as { count: number; filed: string[]; present: string[] };
		assert.deepEqual(outcome.filed, [expected]);
		const active = await listActiveAdvisories(env);
		const advisory = active.find((a) => a.id === expected);
		assert.ok(advisory, 'filed advisory is active');
		assert.equal(advisory.severity, 'critical');
		assert.equal(advisory.package, 'adv-carrier');
		assert.equal(advisory.affected, '2.0.0');
		assert.match(advisory.detail, /registries/);
		assert.match(advisory.detail, /delete the `registry` and `registries` keys/);
		assert.match(advisory.detail, /next version/);
		const again = await fileAdvisories(env);
		const repeated = (await again.json()) as { filed: string[]; present: string[] };
		assert.deepEqual(repeated.filed, []);
		assert.ok(repeated.present.includes(expected ?? ''));
	});

	it('leaves the violator live: advisories inform, they never yank', async () => {
		const env = fake.env;
		const res = await scan(env);
		assert.equal(res.status, 200);
		const body = (await res.json()) as {
			registries: { packages: Array<{ name: string; version: string }> };
		};
		assert.ok(
			body.registries.packages.some((p) => p.name === 'adv-carrier' && p.version === '2.0.0'),
			'violator still scans live'
		);
		const next = await publish(env, 'adv-carrier', '2.0.1', MAIN);
		assert.equal(next.status, 201, 'the name still publishes clean versions');
	});
});
