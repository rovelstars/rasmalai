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

function frame(name: string, version: string, license: string): Record<string, unknown> {
	return {
		name,
		version,
		files: [
			{
				path: 'Project.config',
				content: `export default {\n    project: {\n        name: "${name}",\n        version: "${version}"\n    },\n    entries: { main: "src/main.rnx" }\n}\n`
			},
			{ path: 'LICENSE', content: 'MIT License\n' },
			{ path: 'src/main.rnx', content: 'export fn main(): Void {\n  let x = 1;\n}\n' }
		],
		description: 'fixture',
		readme: `# ${name}\n`,
		license
	};
}

describe('publish license validation', () => {
	it('rejects an unknown SPDX id', async () => {
		const res = await publishPOST({
			request: request(frame('lic-unknown', '1.0.0', 'GPL-2.0')),
			platform: { env: fake.env },
			url: new URL('https://registry.test/api/v0/packages')
		} as never);
		assert.equal(res.status, 400);
		const body = (await res.json()) as { error?: string };
		assert.match(body.error ?? '', /unknown license/);
	});

	it('accepts a known SPDX id', async () => {
		const res = await publishPOST({
			request: request(frame('lic-known', '1.0.0', 'Apache-2.0')),
			platform: { env: fake.env },
			url: new URL('https://registry.test/api/v0/packages')
		} as never);
		assert.equal(res.status, 201);
	});
});
