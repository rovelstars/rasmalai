import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { normalizeGuideEntry, parseStoredGuides } from '../src/lib/server/guides.ts';
import { sanitizeGuideHtml } from '../src/lib/server/sanitize.ts';
import { GET } from '../src/routes/api/v0/packages/[...path]/+server.ts';
import { startFakeLibsql, type FakeLibsql } from './libsql-fake.js';
import { seedPackage } from './publish-helpers.ts';

// The client frame carries raw markdown source only: slug, title, source,
// description, section, order. The server renders sanitized html and the
// package page injects that html, never the raw source.

function clientEntry(source: string) {
	return {
		slug: 'getting-started',
		title: 'Getting Started',
		source,
		description: 'First steps.',
		section: 'Basics',
		order: '1'
	};
}

describe('client guides frame contract', () => {
	it('accepts the exact shape the publish frame sends', () => {
		const r = normalizeGuideEntry(clientEntry('# Getting Started\n\nHello.\n'));
		assert.equal(r.ok, true);
		if (!r.ok) return;
		assert.equal(r.entry.slug, 'getting-started');
		assert.equal(r.entry.source, '# Getting Started\n\nHello.\n');
		assert.equal(r.entry.description, 'First steps.');
		assert.equal(r.entry.section, 'Basics');
		assert.equal(r.entry.order, '1');
		assert.ok(r.entry.html.includes('Getting Started'));
	});

	it('accepts null section and order for guides without frontmatter', () => {
		const r = normalizeGuideEntry({
			slug: 'perf',
			title: 'Perf',
			source: '# Perf\n\nGo fast.\n',
			description: '',
			section: null,
			order: null
		});
		assert.equal(r.ok, true);
		if (!r.ok) return;
		assert.equal(r.entry.section, null);
		assert.equal(r.entry.order, null);
	});

	it('rejects client html and renders from source instead', () => {
		const r = normalizeGuideEntry({
			...clientEntry('# Hi\n'),
			html: '<script>alert(1)</script>'
		} as unknown as Record<string, unknown>);
		assert.equal(r.ok, true);
		if (!r.ok) return;
		assert.ok(!r.entry.html.includes('<script>'));
	});

	it('sanitizes hostile markdown at intake and at serve', () => {
		const evil = '# Hi\n\n<script>alert(1)</script>\n\n[click](javascript:alert(1))\n';
		const r = normalizeGuideEntry(clientEntry(evil));
		assert.equal(r.ok, true);
		if (!r.ok) return;
		assert.ok(!r.entry.html.includes('<script>'), r.entry.html);
		assert.ok(!r.entry.html.includes('javascript:'), r.entry.html);
		assert.ok(r.entry.html.includes('Hi'), r.entry.html);
		const served = sanitizeGuideHtml(r.entry.source);
		assert.ok(!served.includes('<script>'), served);
	});
});

let fake: FakeLibsql;

before(async () => {
	fake = await startFakeLibsql();
});

after(async () => {
	await fake.close();
});

function get(path: string, format?: string): Promise<Response> {
	const suffix = format ? `?format=${format}` : '';
	return GET({
		params: { path },
		url: new URL(`https://registry.test/api/v0/packages/${path}${suffix}`),
		platform: { env: fake.env },
		setHeaders: () => {}
	} as never);
}

describe('package guides routes', () => {
	it('lists guides and serves one guide as html or sanitized source', async () => {
		const first = normalizeGuideEntry(clientEntry('# Getting Started\n\nHello.\n'));
		const second = normalizeGuideEntry({
			slug: 'perf',
			title: 'Perf',
			source: '# Perf\n\nGo fast.\n',
			description: '',
			section: null,
			order: null
		});
		assert.equal(first.ok, true);
		assert.equal(second.ok, true);
		if (!first.ok || !second.ok) return;
		const guidesJson = JSON.stringify([first.entry, second.entry]);
		await seedPackage(fake.env, '@acme/guides', '1.0.0', { guidesJson });

		const index = await get('@acme/guides@1.0.0/guides');
		assert.equal(index.status, 200);
		const listed = (await index.json()) as { guides: Array<{ slug: string; title: string }> };
		assert.deepEqual(
			listed.guides.map((g) => g.slug),
			['getting-started', 'perf']
		);
		assert.ok(!('html' in listed.guides[0]), 'index stays metadata-only');

		const one = await get('@acme/guides@1.0.0/guides/getting-started');
		assert.equal(one.status, 200);
		const entry = (await one.json()) as { slug: string; format: string; html: string };
		assert.equal(entry.format, 'html');
		assert.ok(entry.html.includes('Getting Started'));
		assert.ok(!entry.html.includes('<script>'));

		const src = await get('@acme/guides@1.0.0/guides/getting-started', 'source');
		assert.equal(src.status, 200);
		const raw = (await src.json()) as { slug: string; source: string };
		assert.ok(raw.source.includes('Hello.'));
		assert.ok(!raw.source.includes('<script>'));
	});

	it('answers 404 with an empty-state message when a version ships no guides', async () => {
		await seedPackage(fake.env, '@acme/noguides', '1.0.0');
		assert.deepEqual(parseStoredGuides('[]'), []);
		const res = await get('@acme/noguides@1.0.0/guides');
		assert.equal(res.status, 404);
		const body = (await res.json()) as { message: string };
		assert.match(body.message, /ships no guides/);
		const missing = await get('@acme/noguides@1.0.0/guides/anything');
		assert.equal(missing.status, 404);
	});
});
