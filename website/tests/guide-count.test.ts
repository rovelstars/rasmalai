import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildChapters } from '../src/lib/docs/chapters.ts';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

describe('guide chapter count parity', () => {
	it('landing meta carries no hard-coded chapter count', () => {
		const svelte = readFileSync(join(root, 'src/routes/guide/+page.svelte'), 'utf8');
		assert.doesNotMatch(svelte, /\b(seven|eight|nine|ten|\d+) chapters?\b/);
	});

	it('nav carries no hard-coded chapter or section slugs', () => {
		const nav = readFileSync(join(root, 'src/lib/docs/nav.ts'), 'utf8');
		const block = nav.slice(nav.indexOf('GUIDE_CHAPTERS'), nav.indexOf('LEGACY_GUIDE_REDIRECTS'));
		assert.doesNotMatch(block, /slug: '[^']+'/);
		assert.match(nav, /buildChapters\(guideFiles/);
		assert.match(nav, /buildChapters\(manualFiles/);
	});
});

describe('buildChapters', () => {
	const md = (title: string, extra = '') =>
		`---\ntitle: "${title}"\ndescription: "d"\n${extra}---\n\n# ${title}\n`;

	it('derives slug, title, path in filename order', () => {
		const out = buildChapters(
			{
				'/src/content/guide/02-b.md': md('Bee'),
				'/src/content/guide/01-a.md': md('Aye')
			},
			'/guide'
		);
		assert.deepEqual(
			out.map((c) => [c.slug, c.title, c.path]),
			[
				['01-a', 'Aye', '/guide/01-a'],
				['02-b', 'Bee', '/guide/02-b']
			]
		);
	});

	it('maps section to part and passes icon through', () => {
		const out = buildChapters(
			{ '/src/content/manual/01-x.md': md('Ex', 'section: "Part One"\nicon: "Rocket"\n') },
			'/manual'
		);
		assert.equal(out[0].part, 'Part One');
		assert.equal(out[0].icon, 'Rocket');
	});

	it('honors explicit order over filenames and skips nested files', () => {
		const out = buildChapters(
			{
				'/src/content/guide/01-a.md': md('Aye', 'order: "2"\n'),
				'/src/content/guide/02-b.md': md('Bee', 'order: "1"\n'),
				'/src/content/guide/sub/03-c.md': md('Cee')
			},
			'/guide'
		);
		assert.deepEqual(out.map((c) => c.slug), ['02-b', '01-a']);
	});

	it('throws on missing title', () => {
		assert.throws(() => buildChapters({ '/src/content/guide/01-a.md': 'no frontmatter\n' }, '/guide'), /missing frontmatter title/);
	});

	it('derived guide nav matches the files on disk', () => {
		const dir = join(root, 'src/content/guide');
		const files: Record<string, string> = Object.fromEntries(
			readdirSync(dir)
				.filter((f) => f.endsWith('.md'))
				.map((f) => [`/src/content/guide/${f}`, readFileSync(join(dir, f), 'utf8')])
		);
		const out = buildChapters(files, '/guide');
		const onDisk = readdirSync(dir)
			.filter((f) => /^\d\d-.*\.md$/.test(f))
			.map((f) => f.replace(/\.md$/, ''))
			.sort();
		assert.deepEqual(out.map((c) => c.slug).sort(), onDisk);
		assert.ok(out.every((c) => c.title && c.icon));
	});
});
