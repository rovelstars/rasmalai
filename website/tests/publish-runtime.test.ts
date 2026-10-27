import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { POST as publishPOST } from '../src/routes/api/v0/packages/+server.ts';

let fake: FakeLibsql;

before(async () => {
	fake = await startFakeLibsql();
});

after(async () => {
	await fake.close();
});

function request(body: Record<string, unknown>): Request {
	return new Request('https://registry.test/api/v0/packages', {
		method: 'POST',
		headers: {
			'content-type': 'application/json',
			'X-RNX-Protocol': '0',
			authorization: 'Bearer preview-token-123'
		},
		body: JSON.stringify(body)
	});
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
	return publishPOST({
		request: request(frame(name, version, main, extraConfig)),
		platform: { env: fake.env },
		url: new URL('https://registry.test/api/v0/packages')
	} as never);
}

function reportOnlyOf(body: Record<string, unknown>): {
	registries: { keys: string[] };
} {
	const report = body['reportOnly'] as {
		registries?: { keys?: string[] };
	};
	return {
		registries: { keys: report?.registries?.keys ?? [] }
	};
}

describe('publish runtime-internal rule (rejection)', () => {
	it('rejects bare __rnx_ references before anything is stored', async () => {
		const res = await publish(
			'runtime-bare',
			'1.0.0',
			'fn Main(): Int {\n    return __rnx_clock_mono();\n}\n'
		);
		assert.equal(res.status, 400);
		const body = (await res.json()) as Record<string, unknown>;
		assert.match(String(body['error'] ?? ''), /__rnx_clock_mono/);
	});

	it('rejects bare __rnx_ uses in files the entry graph never reaches', async () => {
		const body = frame(
			'runtime-unreached',
			'1.0.0',
			'fn Main(): Int {\n    return 0;\n}\n'
		) as { files: Array<{ path: string; content: string }> };
		body.files.push({
			path: 'src/stray.rnx',
			content: 'export fn stray(): Int {\n    return __rnx_clock_mono();\n}\n'
		});
		const res = await publishPOST({
			request: request(body),
			platform: { env: fake.env },
			url: new URL('https://registry.test/api/v0/packages')
		} as never);
		assert.equal(res.status, 400);
		const published = (await res.json()) as Record<string, unknown>;
		assert.match(String(published['error'] ?? ''), /__rnx_clock_mono/);
	});

	it('rejects declared from-runtime imports outside @std frames, naming the wrapper', async () => {
		const res = await publish(
			'runtime-declared',
			'1.0.0',
			'import { fn __rnx_clock_mono(): Int } from runtime "time";\nfn Main(): Int {\n    return __rnx_clock_mono();\n}\n'
		);
		assert.equal(res.status, 400);
		const body = (await res.json()) as Record<string, unknown>;
		assert.match(String(body['error'] ?? ''), /__rnx_clock_mono/);
		assert.match(String(body['error'] ?? ''), /@std\/time/);
	});

	it('accepts declared from-native aliases under __ffi_ with no report', async () => {
		const res = await publish(
			'runtime-ffi',
			'1.0.0',
			'import { fn clock(): Int as __ffi_clock } from native "rt"\nfn Main(): Int {\n    return __ffi_clock();\n}\n',
			',\n    permissions: [{ perm: "unsafe:ffi", reason: "test declares its foreign import" }]'
		);
		assert.equal(res.status, 201);
		const body = (await res.json()) as Record<string, unknown>;
		const report = reportOnlyOf(body);
		assert.deepEqual(report.registries.keys, []);
		assert.ok(!('s103' in (body['reportOnly'] as Record<string, unknown> ?? {})));
	});

	it('accepts declared runtime imports in @std frames (org token path)', async () => {
		const res = await publish(
			'@std/probe103',
			'1.0.0',
			'import { fn __rnx_clock_mono(): Int } from runtime "time";\nfn tick(): Int {\n    return __rnx_clock_mono();\n}\nfn Main(): Int {\n    return tick();\n}\n'
		);
		assert.equal(res.status, 201);
	});
});
