import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { sanitizeGuideHtml } from '../src/lib/server/sanitize.js';

describe('guide sanitizer', () => {
	it('keeps the safe set intact', () => {
		const html = '<h2>T</h2><p>Hello <strong>world</strong> and <code>x()</code></p><ul><li><a href="https://example.com">up</a></li></ul><pre>p</pre><blockquote>q</blockquote><hr><table><tr><td>c</td></tr></table><img src="https://example.com/i.png" alt="i"><br>';
		assert.equal(sanitizeGuideHtml(html), html);
	});

	it('strips script elements with their content', () => {
		assert.equal(sanitizeGuideHtml('<p>ok</p><script>alert(1)</script>'), '<p>ok</p>');
		assert.equal(sanitizeGuideHtml('<SCRIPT SRC="x.js"></SCRIPT><p>ok</p>'), '<p>ok</p>');
	});

	it('drops event-handler attributes but keeps the tag', () => {
		assert.equal(
			sanitizeGuideHtml('<p onclick="evil()" onmouseover="x">hi</p>'),
			'<p>hi</p>'
		);
		assert.equal(
			sanitizeGuideHtml('<img src="https://example.com/i.png" onerror="evil()">'),
			'<img src="https://example.com/i.png">'
		);
	});

	it('drops svg/onload vectors entirely', () => {
		assert.equal(sanitizeGuideHtml('<svg onload="evil()"><circle/>hi</svg>'), 'hi');
	});

	it('rejects javascript: and data: URLs', () => {
		assert.equal(sanitizeGuideHtml('<a href="javascript:alert(1)">x</a>'), '<a>x</a>');
		assert.equal(sanitizeGuideHtml('<a href="JaVaScRiPt:alert(1)">x</a>'), '<a>x</a>');
		assert.equal(sanitizeGuideHtml('<img src="data:image/png;base64,AAA">'), '<img>');
		assert.equal(sanitizeGuideHtml('<a href="mailto:a@b.c">m</a>'), '<a href="mailto:a@b.c">m</a>');
	});

	it('drops style attributes and comments', () => {
		assert.equal(
			sanitizeGuideHtml('<p style="x:expression(alert(1))">hi</p><!-- secret -->'),
			'<p>hi</p>'
		);
	});

	it('unwraps unknown tags but keeps their text', () => {
		assert.equal(sanitizeGuideHtml('<div><span>text</span></div>'), 'text');
		assert.equal(sanitizeGuideHtml('<iframe src="https://x"></iframe>after'), 'after');
	});

	it('emits a stray < literally', () => {
		assert.equal(sanitizeGuideHtml('a < b'), 'a < b');
		assert.equal(sanitizeGuideHtml('a < b and <c>'), 'a < b and ');
		assert.equal(sanitizeGuideHtml('<p>a < b</p>'), '<p>a < b</p>');
	});

	it('requires // after http:/https:', () => {
		assert.equal(sanitizeGuideHtml('<a href="https:evil.com">x</a>'), '<a>x</a>');
		assert.equal(sanitizeGuideHtml('<a href="http:evil.com">x</a>'), '<a>x</a>');
		assert.equal(sanitizeGuideHtml('<a href="https://example.com">x</a>'), '<a href="https://example.com">x</a>');
	});

	it('allows same-origin #anchor and /path hrefs', () => {
		assert.equal(sanitizeGuideHtml('<a href="#toc">x</a>'), '<a href="#toc">x</a>');
		assert.equal(sanitizeGuideHtml('<a href="/packages/foo">x</a>'), '<a href="/packages/foo">x</a>');
	});

	it('keeps table sections and h5/h6', () => {
		const html = '<table><caption>c</caption><thead><tr><th>h</th></tr></thead><tbody><tr><td>c</td></tr></tbody><tfoot><tr><td>f</td></tr></tfoot></table><h5>a</h5><h6>b</h6>';
		assert.equal(sanitizeGuideHtml(html), html);
	});
});
