import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { gzipSync } from 'node:zlib';
import { normalizeDocJson, normalizeGuideEntry, parseStoredGuides, parseApiModules } from '../src/lib/server/guides.js';
import { MAX_DOC_JSON_BYTES } from '../src/lib/server/db.js';
import {
	parseManifestPermissions,
	detectCapabilityUse,
	scanTarballCapabilities,
	checkCapabilityCoverage
} from '../src/lib/server/permissions.js';
import { rebuildTarball } from '../src/lib/server/chunks.js';
import { sha256Hex } from '../src/lib/server/chunks.js';

function sampleDocs(): Record<string, unknown> {
	return {
		modules: [
			{
				name: '@acme/ui',
				path: 'src/lib.rnx',
				docs: {
					description: 'UI kit',
					tags: [{ kind: 'param', name: 'x', text: 'why' }]
				},
				functions: [
					{
						name: 'render',
						sig: 'fn render(x: Int): String',
						docs: { description: 'Renders.', tags: [] }
					}
				],
				classes: [
					{
						name: 'Widget',
						docs: { description: '', tags: [] },
						init: null,
						fields: [{ name: 'w', ty: 'Int', docs: { description: '', tags: [] } }],
						methods: []
					}
				],
				enums: [
					{
						name: 'Color',
						docs: { description: '', tags: [] },
						variants: [{ name: 'Red', payload: [], docs: { description: '', tags: [] } }]
					}
				],
				constants: [{ name: 'VERSION', ty: 'String', docs: { description: '', tags: [] } }],
				html: '<script>alert(1)</script>'
			}
		]
	};
}

describe('normalizeDocJson', () => {
	it('accepts compiler docgen shape and round-trips through the /api reader', () => {
		for (const input of [sampleDocs(), JSON.stringify(sampleDocs())]) {
			const r = normalizeDocJson(input);
			assert.equal(r.ok, true);
			if (!r.ok) return;
			assert.ok(r.docJson.length <= MAX_DOC_JSON_BYTES);
			assert.deepEqual(parseApiModules(r.docJson), [{ name: '@acme/ui' }]);
			const back = JSON.parse(r.docJson) as { modules: unknown[] };
			assert.equal(back.modules.length, 1);
		}
	});

	it('drops unknown fields including pre-rendered html', () => {
		const r = normalizeDocJson(sampleDocs());
		assert.equal(r.ok, true);
		if (!r.ok) return;
		assert.ok(!r.docJson.includes('<script>'));
		assert.ok(!r.docJson.includes('"html"'));
	});

	it('strips control characters from text fields', () => {
		const docs = sampleDocs();
		const mod = (docs.modules as Record<string, unknown>[])[0];
		mod.docs = { description: 'a\x00b\x07c', tags: [] };
		const r = normalizeDocJson(docs);
		assert.equal(r.ok, true);
		if (!r.ok) return;
		assert.ok(r.docJson.includes('abc'));
	});

	it('accepts an empty module list', () => {
		const r = normalizeDocJson({ modules: [] });
		assert.equal(r.ok, true);
		if (r.ok) assert.equal(r.docJson, '{"modules":[]}');
	});

	it('rejects malformed payloads with 400', () => {
		const bad: unknown[] = [
			42,
			'not json',
			null,
			[],
			{},
			{ modules: {} },
			{ modules: [null] },
			{ modules: [{ name: '', path: 'x', docs: { description: '', tags: [] }, functions: [], classes: [], enums: [], constants: [] }] },
			{ modules: [{ name: 'm', docs: { description: '', tags: [] }, functions: [], classes: [], enums: [], constants: [] }] },
			{ modules: [{ name: 'm', path: 'p', docs: { description: '', tags: [{ kind: 7 }] }, functions: [], classes: [], enums: [], constants: [] }] },
			{ modules: [{ name: 'm', path: 'p', docs: { description: '', tags: [] }, functions: [{ name: 'f' }], classes: [], enums: [], constants: [] }] }
		];
		for (const input of bad) {
			const r = normalizeDocJson(input);
			assert.equal(r.ok, false, JSON.stringify(input)?.slice(0, 80));
			if (!r.ok) assert.equal(r.status, 400);
		}
	});

	it('rejects oversized payloads with 413 under the storage cap', () => {
		const huge = 'x'.repeat(MAX_DOC_JSON_BYTES + 1);
		const r = normalizeDocJson(huge);
		assert.equal(r.ok, false);
		if (!r.ok) assert.equal(r.status, 413);
		const mods = [];
		for (let i = 0; i < 20; i++) {
			mods.push({
				name: `m${i}`,
				path: 'src/lib.rnx',
				docs: { description: 'y'.repeat(60 * 1024), tags: [] },
				functions: [],
				classes: [],
				enums: [],
				constants: []
			});
		}
		const r2 = normalizeDocJson({ modules: mods });
		assert.equal(r2.ok, false);
		if (!r2.ok) assert.equal(r2.status, 413);
	});
});

describe('publish docs round-trip (publish side -> GET readers)', () => {
	it('stored docJson serves through the /api shape and guides through the /guides shape', () => {
		const r = normalizeDocJson(sampleDocs());
		assert.equal(r.ok, true);
		if (!r.ok) return;
		const api = JSON.parse(r.docJson) as { modules: Array<{ name: string }> };
		assert.deepEqual(
			api.modules.map((m) => m.name),
			['@acme/ui']
		);
		const g = normalizeGuideEntry({ slug: 'start', title: 'Start', source: '# Hi' });
		assert.equal(g.ok, true);
		if (!g.ok) return;
		const stored = JSON.stringify([g.entry]);
		assert.deepEqual(
			parseStoredGuides(stored).map((e) => e.slug),
			['start']
		);
	});
});

describe('detectCapabilityUse', () => {
	it('detects every domain head from real call shapes', () => {
		const out = detectCapabilityUse([
			{ name: 'src/a.rnx', text: 'let t = fs.readText("f");\nlet h = await fetch("https://x");' },
			{ name: 'src/b.rnx', text: 'let c = Process.spawn("ffmpeg", []);\nlet e = Env.get("HOME") ?? "";' },
			{ name: 'src/c.rnx', text: 'print("hi");\nunsafe {\n  let p = 1;\n}' },
			{ name: 'src/d.rnx', text: 'import z from "native";\nlet q = from native "z";' }
		]);
		const heads = new Set(out.map((e) => `${e.file}:${e.head}`));
		assert.ok(heads.has('src/a.rnx:fs'));
		assert.ok(heads.has('src/a.rnx:net'));
		assert.ok(heads.has('src/b.rnx:sys'));
		assert.ok(heads.has('src/b.rnx:env'));
		assert.ok(heads.has('src/c.rnx:term'));
		assert.ok(heads.has('src/c.rnx:unsafe'));
		assert.ok(heads.has('src/d.rnx:unsafe'));
	});

	it('ignores comments, string literals, declarations, and non-rnx files', () => {
		const out = detectCapabilityUse([
			{
				name: 'src/lib.rnx',
				text: [
					'//! print(fs.readText("demo"));',
					'// let x = fetch("https://x");',
					'/* File.open("y"); */',
					'async fn fetch(url: String): String { throw "fetch: failed"; }',
					'class RequestOptions { }',
					'let msg = "call Process.spawn now";',
					'export fn load(): String { return "ok"; }'
				].join('\n')
			},
			{ name: 'README.md', text: 'print("hi");' },
			{ name: 'Project.config', text: 'permissions: ["term:write"]' }
		]);
		assert.deepEqual(out, []);
	});

	it('still sees through qualified receivers on the fallback suffixes', () => {
		const out = detectCapabilityUse([{ name: 'src/lib.rnx', text: 'let s = doc.readText();' }]);
		assert.deepEqual(
			out.map((e) => e.head),
			['term']
		);
	});
});

describe('checkCapabilityCoverage', () => {
	it('passes when every used head is declared, even over-declared', () => {
		const declared = parseManifestPermissions(
			JSON.stringify({ permissions: ['term:write', { perm: 'fs:read:/data', reason: 'seed' }] })
		);
		const used = detectCapabilityUse([
			{ name: 'src/lib.rnx', text: 'print("hi");\nlet t = fs.readText("f");' }
		]);
		assert.equal(checkCapabilityCoverage(declared, used).ok, true);
	});

	it('fails with an actionable diagnostic naming the missing head', () => {
		const declared = parseManifestPermissions(JSON.stringify({ permissions: ['term:write'] }));
		const used = detectCapabilityUse([{ name: 'src/lib.rnx', text: 'let t = fs.readText("f");' }]);
		const r = checkCapabilityCoverage(declared, used);
		assert.equal(r.ok, false);
		if (!r.ok) {
			assert.match(r.error, /fs/);
			assert.match(r.error, /Project\.config permissions/);
		}
	});

	it('passes pure code with no declarations', () => {
		const r = checkCapabilityCoverage([], detectCapabilityUse([{ name: 'src/lib.rnx', text: 'export fn add(a: Int, b: Int): Int { return a + b; }' }]));
		assert.equal(r.ok, true);
	});
});

describe('scanTarballCapabilities', () => {
	async function tarOf(files: Array<{ name: string; text: string }>, gzip: boolean): Promise<Uint8Array> {
		const enc = new TextEncoder();
		const units: Array<{ hash: string; bytes: Uint8Array }> = [];
		const entries = [];
		for (const f of files) {
			const bytes = enc.encode(f.text);
			const hash = await sha256Hex(bytes);
			units.push({ hash, bytes });
			entries.push({ name: f.name, size: bytes.length, dir: false, chunks: [hash] });
		}
		const byHash = new Map(units.map((u) => [u.hash, u.bytes] as const));
		const tar = rebuildTarball(entries, byHash);
		return gzip ? gzipSync(tar) : tar;
	}

	it('scans plain and gzipped tarballs, skipping non-rnx entries', async () => {
		const files = [
			{ name: 'src/lib.rnx', text: 'export fn main(): Void {\n  print("hi");\n}' },
			{ name: 'README.md', text: 'fetch("https://x");' }
		];
		for (const gzip of [false, true]) {
			const out = await scanTarballCapabilities(await tarOf(files, gzip));
			assert.deepEqual(out.map((e) => `${e.file}:${e.head}`), ['src/lib.rnx:term']);
		}
	});

	it('returns no evidence for pure tarballs', async () => {
		const out = await scanTarballCapabilities(
			await tarOf([{ name: 'src/lib.rnx', text: 'export fn add(a: Int, b: Int): Int { return a + b; }' }], false)
		);
		assert.deepEqual(out, []);
	});

	it('skips the scan instead of inflating a gzip bomb', async () => {
		const bomb = gzipSync(Buffer.alloc(5 * 1024 * 1024, 'a'));
		assert.ok(bomb.length <= 512 * 1024);
		const out = await scanTarballCapabilities(new Uint8Array(bomb));
		assert.deepEqual(out, []);
	});
});

describe('stdlib capability baseline', () => {
	const STD = join(import.meta.dirname, '..', '..', 'compiler', 'stdlib', 'src');

	it('only fs, io, and process use observable sinks', () => {
		const got = new Map<string, string[]>();
		for (const e of readdirSync(STD, { withFileTypes: true })) {
			if (!e.isFile() || !e.name.endsWith('.rnx')) continue;
			const name = e.name.slice(0, -4);
			const text = readFileSync(join(STD, e.name), 'utf8');
			const heads = [...new Set(detectCapabilityUse([{ name: `src/${name}.rnx`, text }]).map((u) => u.head))].sort();
			got.set(name, heads);
		}
		const sub = readFileSync(join(STD, 'net', 'http.rnx'), 'utf8');
		assert.deepEqual(detectCapabilityUse([{ name: 'src/http.rnx', text: sub }]), []);
		for (const [name, heads] of got) {
			if (name === 'fs') assert.deepEqual(heads, ['unsafe'], name);
			else if (name === 'io') assert.deepEqual(heads, ['env', 'term'], name);
			else if (name === 'process') assert.deepEqual(heads, ['env'], name);
			else assert.deepEqual(heads, [], name);
		}
	});
});
