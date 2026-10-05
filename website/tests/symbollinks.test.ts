import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import type { DocModule } from '../src/lib/docs/api.ts';
import { anchorFor, buildSymbolIndex, linkifyProseHtml } from '../src/lib/docs/symbollinks.ts';

function mod(name: string): DocModule {
	return {
		name,
		docs: { description: '', tags: [] },
		functions: [{ name: 'statusCode', sig: 'fn statusCode()', docs: { description: '', tags: [] } }],
		classes: [{ name: 'Map', docs: { description: '', tags: [] }, init: null, fields: [], methods: [] }],
		enums: [{ name: 'HttpStatus', docs: { description: '', tags: [] }, variants: [] }],
		constants: [{ name: 'VERSION', docs: { description: '', tags: [] }, ty: 'String' }]
	};
}

describe('anchorFor', () => {
	it('uses the ModuleDocs id scheme', () => {
		assert.equal(anchorFor('class', 'Map'), 'class-Map');
		assert.equal(anchorFor('enum', 'HttpStatus'), 'enum-HttpStatus');
		assert.equal(anchorFor('fn', 'run'), 'fn-run');
		assert.equal(anchorFor('const', 'VERSION'), 'const-VERSION');
	});
});

describe('buildSymbolIndex', () => {
	it('maps every export to its docs anchor', () => {
		const index = buildSymbolIndex([mod('collections')]);
		assert.equal(index.get('Map'), '/docs/@std/collections/api#class-Map');
		assert.equal(index.get('HttpStatus'), '/docs/@std/collections/api#enum-HttpStatus');
		assert.equal(index.get('statusCode'), '/docs/@std/collections/api#fn-statusCode');
		assert.equal(index.get('VERSION'), '/docs/@std/collections/api#const-VERSION');
	});

	it('keeps the first module on name collisions', () => {
		const index = buildSymbolIndex([mod('a'), mod('b')]);
		assert.equal(index.get('Map'), '/docs/@std/a/api#class-Map');
	});

	it('accepts a custom href builder', () => {
		const index = buildSymbolIndex([mod('x')], (m, a) => `#pkg-${a}`);
		assert.equal(index.get('Map'), '#pkg-class-Map');
	});
});

describe('linkifyProseHtml', () => {
	const index = buildSymbolIndex([mod('collections')]);

	it('links exact TitleCase matches with no color change', () => {
		const out = linkifyProseHtml('<p>Use Map for lookup.</p>', index);
		assert.ok(out.includes('<a href="/docs/@std/collections/api#class-Map" class="symlink">Map</a>'));
	});

	it('does not match partial words', () => {
		assert.equal(linkifyProseHtml('<p>Mapping is fun.</p>', index), '<p>Mapping is fun.</p>');
		assert.equal(linkifyProseHtml('<p>MyMap here.</p>', index), '<p>MyMap here.</p>');
	});

	it('skips lowercase exports and code spans', () => {
		assert.equal(
			linkifyProseHtml('<p>Call statusCode today.</p>', index),
			'<p>Call statusCode today.</p>'
		);
		const code = '<p>See <code>Map</code> for details.</p>';
		assert.equal(linkifyProseHtml(code, index), code);
		const pre = '<pre><code>new Map()</code></pre><p>Map works.</p>';
		assert.ok(linkifyProseHtml(pre, index).includes('<pre><code>new Map()</code></pre>'));
		assert.ok(linkifyProseHtml(pre, index).includes('>Map</a> works.'));
	});

	it('excludes the symbol under definition', () => {
		const out = linkifyProseHtml('<p>Map stores entries.</p>', index, new Set(['Map']));
		assert.equal(out, '<p>Map stores entries.</p>');
	});

	it('leaves html without matches untouched', () => {
		const html = '<p>Nothing to link here.</p>';
		assert.equal(linkifyProseHtml(html, index), html);
		assert.equal(linkifyProseHtml(html, new Map()), html);
	});
});
