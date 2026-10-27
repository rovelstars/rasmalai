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
	s103: { count: number; files: Record<string, string[]> };
	registries: { keys: string[] };
} {
	const report = body['reportOnly'] as {
		s103?: { count?: number; files?: Record<string, string[]> };
		registries?: { keys?: string[] };
	};
	return {
		s103: { count: report?.s103?.count ?? -1, files: report?.s103?.files ?? {} },
		registries: { keys: report?.registries?.keys ?? [] }
	};
}

describe('publish S103 prefix rule (report-only)', () => {
	it('accepts bare __rnx_ references and counts them per file', async () => {
		const lines: string[] = [];
		const orig = console.warn;
		console.warn = (message?: unknown) => {
			lines.push(String(message));
		};
		let res: Response;
		try {
			res = await publish(
				's103-bare',
				'1.0.0',
				'fn Main(): Int {\n    return __rnx_clock_mono();\n}\n'
			);
		} finally {
			console.warn = orig;
		}
		assert.equal(res.status, 201);
		const body = (await res.json()) as Record<string, unknown>;
		const report = reportOnlyOf(body);
		assert.equal(report.s103.count, 1);
		assert.deepEqual(report.s103.files, { 'src/main.rnx': ['__rnx_clock_mono'] });
		assert.deepEqual(report.registries.keys, []);
		const logged = lines.filter((l) => l.includes('"scope":"publish-report-only"'));
		assert.equal(logged.length, 1, JSON.stringify(lines));
		assert.match(logged[0], /s103-bare/);
		assert.match(logged[0], /__rnx_clock_mono/);
	});

	it('accepts bare __rnx_ uses in files the entry graph never reaches, still counted', async () => {
		const body = frame(
			's103-unreached',
			'1.0.0',
			'fn Main(): Int {\n    return 0;\n}\n'
		) as { files: Array<{ path: string; content: string }> };
		body.files.push({
			path: 'src/stray.rnx',
			content: 'export fn stray(): Int {\n    return __rnx_clock_mono();\n}\n'
		});
		const lines: string[] = [];
		const orig = console.warn;
		console.warn = (message?: unknown) => {
			lines.push(String(message));
		};
		let res: Response;
		try {
			res = await publishPOST({
				request: request(body),
				platform: { env: fake.env },
				url: new URL('https://registry.test/api/v0/packages')
			} as never);
		} finally {
			console.warn = orig;
		}
		assert.equal(res.status, 201);
		const published = (await res.json()) as Record<string, unknown>;
		const report = reportOnlyOf(published);
		assert.equal(report.s103.count, 1);
		assert.deepEqual(report.s103.files, { 'src/stray.rnx': ['__rnx_clock_mono'] });
		assert.equal(lines.filter((l) => l.includes('"scope":"publish-report-only"')).length, 1);
	});

	it('accepts declared from-native aliases under __ffi_ with no count', async () => {
		const res = await publish(
			's103-ffi',
			'1.0.0',
			'import { fn clock(): Int as __ffi_clock } from native "rt"\nfn Main(): Int {\n    return __ffi_clock();\n}\n',
			',\n    permissions: [{ perm: "unsafe:ffi", reason: "test declares its foreign import" }]'
		);
		assert.equal(res.status, 201);
		const body = (await res.json()) as Record<string, unknown>;
		const report = reportOnlyOf(body);
		assert.equal(report.s103.count, 0);
		assert.deepEqual(report.s103.files, {});
	});

	it('exempts @std frames by package name (org token path)', async () => {
		const res = await publish(
			'@std/probe103',
			'1.0.0',
			'fn tick(): Int {\n    return __rnx_clock_mono();\n}\nfn Main(): Int {\n    return tick();\n}\n'
		);
		assert.equal(res.status, 201);
		const body = (await res.json()) as Record<string, unknown>;
		const report = reportOnlyOf(body);
		assert.equal(report.s103.count, 0);
	});
});
