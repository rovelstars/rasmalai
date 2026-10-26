import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { normalizeGuideEntry } from '../src/lib/server/guides.ts';
import { seedPackage } from './publish-helpers.js';
import { load as docsLoad } from '../src/routes/docs/[scope]/[name]/[page]/+page.server.ts';
import { load as packageLoad } from '../src/routes/packages/[...slug]/+page.server.ts';
import { load as catalogLoad } from '../src/routes/packages/+page.server.ts';
import { handle } from '../src/hooks.server.ts';

const ROOT = join(import.meta.dirname, '..');

let fake: FakeLibsql;

before(async () => {
	fake = await startFakeLibsql();
});

after(async () => {
	await fake.close();
});

function headers() {
	const seen: Record<string, string> = {};
	return {
		seen,
		setHeaders(h: Record<string, string>) {
			Object.assign(seen, h);
		}
	};
}

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

function docsCall(scope: string, name: string, page: string, h: { setHeaders: (v: Record<string, string>) => void }) {
	return docsLoad({
		params: { scope, name, page },
		platform: { env: fake.env },
		url: new URL(`https://registry.test/docs/${scope}/${name}/${page}`),
		fetch: globalThis.fetch,
		setHeaders: h.setHeaders
	} as never);
}

describe('docs html cache headers', () => {
	it('serves registry docs with the shared short public window', async () => {
		fake.reset();
		await seedPackage(fake.env, '@acme/widgets', '2.0.0', {
			description: 'Widgets for every dashboard.',
			docJson: docJson(),
			guidesJson: guidesJson()
		});
		for (const page of ['overview', 'getting-started', 'api']) {
			const h = headers();
			const data = (await docsCall('@acme', 'widgets', page, h)) as Record<string, unknown>;
			assert.equal(data['full'], '@acme/widgets');
			assert.equal(h.seen['Cache-Control'], 'public, max-age=300, s-maxage=300');
		}
	});

	it('never sets a session cookie on the cached docs path', async () => {
		fake.reset();
		await seedPackage(fake.env, '@acme/plain', '1.0.0', { docJson: docJson() });
		const h = headers();
		await docsCall('@acme', 'plain', 'overview', h);
		assert.equal(Object.keys(h.seen).some((k) => k.toLowerCase() === 'set-cookie'), false);
	});

	it('keeps the docs server load free of per-user branches', () => {
		const src = readFileSync(
			join(ROOT, 'src', 'routes', 'docs', '[scope]', '[name]', '[page]', '+page.server.ts'),
			'utf8'
		);
		for (const token of ['Cookie', 'Set-Cookie', 'sessionUser', 'readSessionCookie']) {
			assert.equal(src.includes(token), false, `docs load must not reference ${token}`);
		}
	});
});

describe('package html cache headers', () => {
	it('keeps latest at the short window and pinned at a day', async () => {
		fake.reset();
		await seedPackage(fake.env, '@acme/gadget', '1.0.0', { docJson: docJson() });
		await seedPackage(fake.env, '@acme/gadget', '1.1.0', { docJson: docJson() });
		const latest = headers();
		await packageLoad({
			params: { slug: '@acme/gadget' },
			platform: { env: fake.env },
			url: new URL('https://registry.test/packages/@acme/gadget'),
			setHeaders: latest.setHeaders
		} as never);
		assert.equal(latest.seen['Cache-Control'], 'public, max-age=300, s-maxage=300');
		const pinned = headers();
		await packageLoad({
			params: { slug: '@acme/gadget@1.1.0' },
			platform: { env: fake.env },
			url: new URL('https://registry.test/packages/@acme/gadget@1.1.0'),
			setHeaders: pinned.setHeaders
		} as never);
		assert.equal(pinned.seen['Cache-Control'], 'public, max-age=86400, s-maxage=86400');
		assert.equal(Object.keys(pinned.seen).some((k) => k.toLowerCase() === 'set-cookie'), false);
	});

	it('keeps the catalog aggregate on its hourly edge window', async () => {
		fake.reset();
		const h = headers();
		await catalogLoad({
			platform: { env: fake.env },
			setHeaders: h.setHeaders
		} as never);
		assert.equal(h.seen['Cache-Control'], 'public, max-age=60, s-maxage=3600');
	});
});

describe('sitemap cache headers', () => {
	it('serves xml with the aggregate window and no session cookie', () => {
		const src = readFileSync(join(ROOT, 'src', 'routes', 'sitemap.xml', '+server.ts'), 'utf8');
		assert.match(src, /Cache-Control/);
		assert.match(src, /public, max-age=60, s-maxage=3600/);
		assert.match(src, /application\/xml/);
		for (const token of ['Set-Cookie', 'Cookie', 'sessionUser']) {
			assert.equal(src.includes(token), false, `sitemap must not reference ${token}`);
		}
	});
});

describe('edge cache allowlist', () => {
	function event(path: string) {
		return {
			url: new URL(`https://registry.test${path}`),
			request: new Request(`https://registry.test${path}`)
		} as never;
	}

	function invoke(path: string, resolve: () => Promise<Response>) {
		return handle({ event: event(path), resolve } as never);
	}

	function stubCache() {
		const store = new Map<string, Response>();
		const stub = {
			async match(key: Request) {
				return store.get(key.url) ?? undefined;
			},
			async put(key: Request, res: Response) {
				store.set(key.url, res.clone());
			}
		};
		(globalThis as Record<string, unknown>)['caches'] = { default: stub };
		return () => {
			delete (globalThis as Record<string, unknown>)['caches'];
		};
	}

	it('caches anonymous docs html through the worker cache', async () => {
		const restore = stubCache();
		try {
			let calls = 0;
			const resolve = async () => {
				calls += 1;
				return new Response('<html>docs</html>', {
					headers: { 'content-type': 'text/html', 'Cache-Control': 'public, max-age=300, s-maxage=300' }
				});
			};
			const first = await invoke('/docs/@std/time/overview', resolve);
			assert.equal(calls, 1);
			assert.equal(first.headers.get('Cache-Control'), 'public, max-age=300, s-maxage=300');
			const second = await invoke('/docs/@std/time/overview', resolve);
			assert.equal(calls, 1);
			assert.equal(await second.text(), '<html>docs</html>');
		} finally {
			restore();
		}
	});

	it('leaves the personalized identity endpoint uncached', async () => {
		const restore = stubCache();
		try {
			let calls = 0;
			const resolve = async () => {
				calls += 1;
				return new Response('{}', { headers: { 'content-type': 'application/json' } });
			};
			await invoke('/api/me', resolve);
			await invoke('/api/me', resolve);
			assert.equal(calls, 2);
		} finally {
			restore();
		}
	});
});
