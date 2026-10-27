import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { POST as publishPOST } from '../src/routes/api/v0/packages/+server.ts';
import { GET as rulesGET } from '../src/routes/api/admin/publish-rules/+server.ts';

let fake: FakeLibsql;

const adminEnv = (): Record<string, string | undefined> => ({ ...fake.env, ADMIN_TOKEN: 'admin' });

before(async () => {
	fake = await startFakeLibsql();
});

after(async () => {
	await fake.close();
});

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
			{
				path: 'Project.config',
				content: `export default {\n    project: {\n        name: "${name}",\n        version: "${version}"\n    },\n    entries: { main: "src/main.rnx" }${extraConfig}\n}\n`
			},
			{ path: 'LICENSE', content: 'MIT License\n' },
			{ path: 'src/main.rnx', content: main }
		],
		description: 'fixture',
		readme: `# ${name}\n`,
		license: 'MIT'
	};
}

async function publish(name: string, version: string, main: string, extraConfig = '') {
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
		platform: { env: fake.env },
		url: new URL('https://registry.test/api/v0/packages')
	} as never);
}

function scan(limit?: number): Promise<Response> {
	const target =
		limit === undefined
			? 'https://registry.test/api/admin/publish-rules'
			: `https://registry.test/api/admin/publish-rules?limit=${limit}`;
	return rulesGET({
		request: new Request(target, { headers: { authorization: 'Bearer admin' } }),
		platform: { env: adminEnv() },
		url: new URL(target)
	} as never);
}

describe('admin publish-rules report-only scan', () => {
	it('rejects runtime internals and counts registry-key carriers without rejecting them', async () => {
		const rejected = await publish(
			'scan-runtime-victim',
			'1.0.0',
			'fn Main(): Int {\n    return __rnx_clock_mono();\n}\n'
		);
		assert.equal(rejected.status, 400);
		const rejectedBody = (await rejected.json()) as { error?: string };
		assert.match(rejectedBody.error ?? '', /__rnx_clock_mono/);
		const carrier = await publish(
			'scan-registry-carrier',
			'1.0.0',
			'fn Main(): Int {\n    return 0;\n}\n',
			',\n    registries: { "@acme": { url: "https://acme.example.invalid" } }'
		);
		assert.equal(carrier.status, 201);
		const carrierBody = (await carrier.json()) as {
			reportOnly?: { registries?: { keys?: string[] } };
		};
		assert.deepEqual(carrierBody.reportOnly?.registries?.keys, ['registries']);
		assert.ok(!('s103' in (carrierBody.reportOnly ?? {})), JSON.stringify(carrierBody.reportOnly));
		const clean = await publish('scan-clean', '1.0.0', 'fn Main(): Int {\n    return 0;\n}\n');
		assert.equal(clean.status, 201);

		const res = await scan();
		assert.equal(res.status, 200);
		const body = (await res.json()) as {
			scanned: number;
			skipped: unknown[];
			truncated: boolean;
			limit: number;
			registries: { count: number; packages: Array<{ name: string; version: string; keys: string[] }> };
		};
		assert.equal(body.scanned, 2);
		assert.deepEqual(body.skipped, []);
		assert.equal(body.truncated, false);
		assert.ok(!('s103' in body), JSON.stringify(Object.keys(body)));
		assert.equal(body.registries.count, 1);
		assert.equal(body.registries.packages.length, 1);
		assert.equal(body.registries.packages[0].name, 'scan-registry-carrier');
		assert.deepEqual(body.registries.packages[0].keys, ['registries']);
	});

	it('refuses the scan without the admin token', async () => {
		const res = await rulesGET({
			request: new Request('https://registry.test/api/admin/publish-rules'),
			platform: { env: adminEnv() },
			url: new URL('https://registry.test/api/admin/publish-rules')
		} as never);
		assert.equal(res.status, 401);
	});

	it('bounds the scan with limit and reports truncation', async () => {
		const res = await scan(1);
		assert.equal(res.status, 200);
		const body = (await res.json()) as { scanned: number; truncated: boolean; limit: number };
		assert.equal(body.limit, 1);
		assert.ok(body.scanned <= 1);
		assert.equal(body.truncated, true);
	});
});
