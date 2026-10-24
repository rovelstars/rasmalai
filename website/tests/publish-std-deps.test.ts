import { describe, it, before, after, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { POST as publishPOST } from '../src/routes/api/v0/packages/+server.ts';

// Cross-package analysis: a publish whose tree imports @std/* must
// analyse against the dependency's published sources (fetched from R2
// by the server), not fail with "not preloaded". Unknown modules must
// refuse the publish instead of resolving to nothing.

let fake: FakeLibsql;

function config(name: string, version: string): string {
	return `export default {\n    project: {\n        name: "${name}",\n        version: "${version}"\n    },\n    entries: { main: "src/lib.rnx" }\n}\n`;
}

function frameOf(
	name: string,
	version: string,
	files: Array<{ path: string; content: string }>
): Record<string, unknown> {
	return {
		name,
		version,
		files: [
			{ path: 'Project.config', content: config(name, version) },
			{ path: 'LICENSE', content: 'MIT License\n' },
			...files
		],
		description: 'fixture',
		readme: `# ${name}\n`,
		license: 'MIT'
	};
}

function publish(name: string, version: string, frame: Record<string, unknown>): Promise<Response> {
	const request = new Request('https://registry.test/api/v0/packages', {
		method: 'POST',
		headers: {
			'X-RNX-Protocol': '0',
			authorization: 'Bearer preview-token-123',
			'Content-Type': 'application/json'
		},
		body: JSON.stringify(frame)
	});
	return publishPOST({
		request,
		url: new URL('https://registry.test/api/v0/packages'),
		platform: { env: fake.env }
	} as never);
}

const PRELUDE = 'export fn ident(x: Int): Int {\n  return x;\n}\n';
const USES = 'import { ident } from "@std/prelude";\nexport fn main(): Void {\n  let y = ident(1);\n}\n';

before(async () => {
	fake = await startFakeLibsql();
});

after(async () => {
	await fake.close();
});

beforeEach(() => {
	fake.reset();
});

describe('publish cross-package analysis', () => {
	it('analyses a tree importing a published @std/* dep', async () => {
		const dep = await publish('@std/prelude', '0.1.0', frameOf('@std/prelude', '0.1.0', [
			{ path: 'src/lib.rnx', content: PRELUDE }
		]));
		assert.equal(dep.status, 201);
		const res = await publish('@std/usespre', '0.1.0', frameOf('@std/usespre', '0.1.0', [
			{ path: 'src/lib.rnx', content: USES }
		]));
		assert.equal(res.status, 201);
		const body = (await res.json()) as { manifestDigest?: string };
		assert.ok(body.manifestDigest && body.manifestDigest.length === 64);
	});

	it('refuses a tree importing an unpublished @std/* dep', async () => {
		const res = await publish('@std/earlybird', '0.1.0', frameOf('@std/earlybird', '0.1.0', [
			{
				path: 'src/lib.rnx',
				content:
					'import { ByteBuffer } from "@std/bytes";\nexport fn main(): Void {\n  let b = ByteBuffer.allocate(8);\n}\n'
			}
		]));
		assert.equal(res.status, 400);
	});

	it('refuses an unknown standard library module', async () => {
		const res = await publish('@std/confused', '0.1.0', frameOf('@std/confused', '0.1.0', [
			{
				path: 'src/lib.rnx',
				content: 'import { nope } from "@std/nosuchmod";\nexport fn main(): Void {\n}\n'
			}
		]));
		assert.equal(res.status, 400);
	});
});
