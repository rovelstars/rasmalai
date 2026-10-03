import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

describe('guide chapter count parity', () => {
	it('landing meta carries no hard-coded chapter count', () => {
		const svelte = readFileSync(join(root, 'src/routes/guide/+page.svelte'), 'utf8');
		assert.doesNotMatch(svelte, /\b(seven|eight|nine|ten|\d+) chapters?\b/);
	});

	it('nav guide entries match the markdown files on disk', () => {
		const nav = readFileSync(join(root, 'src/lib/docs/nav.ts'), 'utf8');
		const block = nav.slice(nav.indexOf('GUIDE_CHAPTERS'), nav.indexOf('ManualMeta'));
		const slugs = [...block.matchAll(/slug: '([^']+)'/g)].map((m) => m[1]);
		const files = readdirSync(join(root, 'src/content/guide'))
			.filter((f) => /^\d\d-.*\.md$/.test(f))
			.map((f) => f.replace(/\.md$/, ''))
			.sort();
		assert.deepEqual([...slugs].sort(), files);
	});
});
