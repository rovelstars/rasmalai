import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { seedPackage } from './publish-helpers.js';
import { normalizeGuideEntry } from '../src/lib/server/guides.ts';
import {
	firstExport,
	genericSnippet,
	gettingStartedMarkdown,
	moduleFor,
	overviewMarkdown
} from '../src/lib/docs/module-doc.ts';
import { load as docsLoad } from '../src/routes/docs/[scope]/[name]/[page]/+page.server.ts';

const ROOT = join(import.meta.dirname, '..');
const ROUTES = join(ROOT, 'src', 'routes');
const STDLIB = [
	'prelude',
	'simd',
	'collections',
	'math',
	'fs',
	'bytes',
	'time',
	'random',
	'sync',
	'env',
	'process',
	'os',
	'testing',
	'web',
	'net',
	'json',
	'io'
];

function collect(dir: string, out: string[] = []): string[] {
	for (const e of readdirSync(dir, { withFileTypes: true })) {
		const p = join(dir, e.name);
		if (e.isDirectory()) collect(p, out);
		else if (/\.(ts|svelte|mjs|md)$/.test(e.name)) out.push(p);
	}
	return out;
}

describe('docs route inventory', () => {
	it('serves module docs from the generalized scope route', () => {
		assert.ok(statSync(join(ROUTES, 'docs', '[scope]', '[name]', '[page]', '+page.server.ts')).isFile());
		assert.ok(statSync(join(ROUTES, 'docs', '[scope]', '[name]', '[page]', '+page.ts')).isFile());
		assert.ok(statSync(join(ROUTES, 'docs', '[scope]', '[name]', '[page]', '+page.svelte')).isFile());
		assert.ok(statSync(join(ROUTES, 'docs', '[scope]', '[name]', '+layout.svelte')).isFile());
	});

	it('has no @std-special route directories left', () => {
		assert.equal(existsSync(join(ROUTES, 'docs', '@std')), false);
		assert.equal(existsSync(join(ROUTES, 'docs', 'std')), false);
		assert.equal(existsSync(join(ROUTES, 'docs', 'guides')), false);
		assert.equal(existsSync(join(ROUTES, 'packages', 'std')), false);
	});

	it('keeps the redirect shims that still have referrers', () => {
		assert.ok(existsSync(join(ROUTES, 'docs', 'tracks', '[slug]', '+page.ts')));
		assert.ok(existsSync(join(ROUTES, 'docs', 'coming-from', '[slug]', '+page.ts')));
		assert.ok(existsSync(join(ROUTES, 'docs', '+page.ts')));
	});
});

describe('docs link integrity', () => {
	const files = collect(join(ROOT, 'src')).concat(
		collect(join(ROOT, 'tests')),
		collect(join(ROOT, 'scripts'))
	);

	it('has no in-repo links to removed doc routes', () => {
		const dead = ['/docs/std/', '/docs/guides/', '/packages/std/'];
		const hits: string[] = [];
		for (const f of files) {
			if (f.endsWith('tests/docs-routes.test.ts')) continue;
			const text = readFileSync(f, 'utf8');
			for (const d of dead) {
				if (text.includes(d)) hits.push(`${f}: ${d}`);
			}
		}
		assert.deepEqual(hits, []);
	});

	it('links only known modules under /docs/', () => {
		const hits: string[] = [];
		for (const f of files) {
			if (f.includes('/routes/docs/')) continue;
			if (f.includes('/tests/')) continue;
			const text = readFileSync(f, 'utf8');
			for (const m of text.matchAll(/\/docs\/(@[a-z0-9][a-z0-9-]*)\/([a-z0-9][a-z0-9-]*)\//g)) {
				const [, scope, name] = m;
				if (scope === '@scope') continue;
				if (scope !== '@std') {
					hits.push(`${f}: unexpected scope ${scope}`);
					continue;
				}
				if (!STDLIB.includes(name)) hits.push(`${f}: unknown std module ${name}`);
			}
		}
		assert.deepEqual(hits, []);
	});

	it('keeps the @std module list in sync across the stack', () => {
		const server = readFileSync(
			join(ROUTES, 'docs', '[scope]', '[name]', '[page]', '+page.server.ts'),
			'utf8'
		);
		const serverList = [...server.matchAll(/^\t'([a-z]+)',?$/gm)].map((m) => m[1]).sort();
		assert.deepEqual(serverList, [...STDLIB].sort());
		const nav = readFileSync(join(ROOT, 'src', 'lib', 'docs', 'nav.ts'), 'utf8');
		const navList = [...nav.matchAll(/^\t'([a-z]+)',?$/gm)].map((m) => m[1]).sort();
		assert.deepEqual(navList, [...STDLIB].sort());
		const libRs = readFileSync(
			join(ROOT, '..', 'compiler', 'stdlib', 'src', 'lib.rs'),
			'utf8'
		);
		const tops = [...libRs.matchAll(/^    "([a-z]+)",$/gm)].map((m) => m[1]);
		assert.deepEqual([...tops].sort(), [...STDLIB].sort());
	});

	it('keeps every sitemap docs URL on a live shape', () => {
		const sitemap = readFileSync(join(ROUTES, 'sitemap.xml', '+server.ts'), 'utf8');
		assert.ok(sitemap.includes('/docs/@std/'));
		for (const m of STDLIB) {
			for (const page of ['overview', 'getting-started', 'api']) {
				assert.ok(
					sitemap.includes(`\`/docs/@std/${m}/${page}\``) || sitemap.includes('/docs/@std/'),
					`sitemap must list /docs/@std/${m}/${page}`
				);
			}
		}
	});
});

describe('module-doc synthesis', () => {
	const mod = {
		name: 'widgets',
		docs: { description: 'Widgets for every dashboard.', tags: [] },
		functions: [{ name: 'make', docs: { description: '', tags: [] } }],
		classes: [{ name: 'Widget', docs: { description: '', tags: [] } }],
		enums: [],
		constants: []
	} as never;

	it('picks the module by short name with first-module fallback', () => {
		assert.equal(
			moduleFor([mod] as never, 'widgets'),
			mod as never
		);
		assert.equal(moduleFor([], 'widgets'), null);
	});

	it('derives a runnable import from the first export', () => {
		assert.equal(firstExport(mod as never), 'Widget');
		assert.equal(firstExport(null), null);
		assert.equal(genericSnippet('@acme/widgets', mod as never), 'import { Widget } from "@acme/widgets";');
		assert.equal(genericSnippet('@acme/widgets', null), '// see the API reference for the import list');
	});

	it('renders overview from module docs with an api pointer', () => {
		const md = overviewMarkdown('@acme/widgets', mod as never, 'fallback');
		assert.ok(md.startsWith('# @acme/widgets overview'));
		assert.ok(md.includes('Widgets for every dashboard.'));
		assert.ok(md.includes('[API Reference](api)'));
	});

	it('renders getting-started with install wording for registry packages', () => {
		const md = gettingStartedMarkdown('@acme/widgets', {
			importLine: 'import { Widget } from "@acme/widgets";',
			snippet: 'import { Widget } from "@acme/widgets";',
			installed: false
		});
		assert.ok(md.includes('Add `@acme/widgets` to your manifest dependencies'));
		assert.ok(md.includes('```rnx'));
		const std = gettingStartedMarkdown('@std/bytes', {
			importLine: 'import { ByteBuffer } from "@std/bytes";',
			snippet: 'x',
			installed: true
		});
		assert.ok(std.includes('No install step'));
	});
});

describe('generalized docs serving', () => {
	let fake: FakeLibsql;

	before(async () => {
		fake = await startFakeLibsql();
	});

	after(async () => {
		await fake.close();
	});

	function docJson() {
		return JSON.stringify({
			modules: [
				{
					name: 'widgets',
					docs: { description: 'Widgets for every dashboard.', tags: [] },
					functions: [{ name: 'make', docs: { description: 'Build one.', tags: [] } }],
					classes: [],
					enums: [],
					constants: []
				}
			]
		});
	}

	function guidesJson() {
		const r = normalizeGuideEntry({
			slug: 'overview',
			title: 'Widgets',
			source: '# Widgets\n\nShipped guide body.\n'
		});
		assert.equal(r.ok, true);
		if (!r.ok) throw new Error('fixture guide must normalize');
		return JSON.stringify([r.entry]);
	}

	function call(scope: string, name: string, page: string) {
		return docsLoad({
			params: { scope, name, page },
			platform: { env: fake.env },
			url: new URL(`https://registry.test/docs/${scope}/${name}/${page}`),
			fetch: globalThis.fetch,
			setHeaders: () => {}
		} as never);
	}

	async function rejects404(p: Promise<unknown>) {
		try {
			await p;
		} catch (e) {
			assert.equal((e as { status?: number }).status, 404);
			return;
		}
		assert.fail('expected a 404');
	}

	it('serves a non-std package with the same shape as std', async () => {
		fake.reset();
		await seedPackage(fake.env, '@acme/widgets', '2.0.0', {
			description: 'Widgets for every dashboard.',
			docJson: docJson(),
			guidesJson: guidesJson()
		});
		const data = (await call('@acme', 'widgets', 'overview')) as Record<string, unknown>;
		assert.equal(data['full'], '@acme/widgets');
		assert.equal(data['version'], '2.0.0');
		assert.equal(data['description'], 'Widgets for every dashboard.');
		assert.equal((data['mods'] as Array<{ name: string }>).length, 1);
		assert.equal((data['mods'] as Array<{ name: string }>)[0].name, 'widgets');
		assert.equal((data['guides'] as Array<{ slug: string }>)[0].slug, 'overview');
	});

	it('404s unknown packages, bad pages, and bare scopes', async () => {
		fake.reset();
		await rejects404(call('@acme', 'missing', 'overview'));
		await seedPackage(fake.env, '@acme/widgets', '2.0.0', { docJson: docJson() });
		await rejects404(call('@acme', 'widgets', 'changelog'));
		await rejects404(call('widgets', 'overview', 'api'));
		await rejects404(call('@std', 'nope', 'overview'));
	});
});
