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

	it('emits a stray < literally when no tag can follow', () => {
		assert.equal(sanitizeGuideHtml('a < b'), 'a < b');
		assert.equal(sanitizeGuideHtml('a < b and <c>'), 'a < b and ');
		assert.equal(sanitizeGuideHtml('<p>a < b</p>'), '<p>a < b</p>');
		assert.equal(sanitizeGuideHtml('5 < /regex/ here'), '5 < /regex/ here');
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

// Vectors where the sanitizer's tokenizer and a browser's HTML parser disagree
// about where a tag ends. Asserting on the sanitized string is enough: output
// with no handler attribute, no foreign-content element, and no unterminated
// tag cannot be reparsed into something executable.
describe('parser-differential vectors', () => {
	it('neutralizes the noscript reparse vector', () => {
		// With scripting on, a browser reads <noscript> content as raw text, so
		// '<noscript><p>' hides the element boundary and the img after
		// '</noscript>' is a real element. dropDepth eats the <p>.
		assert.equal(
			sanitizeGuideHtml('<noscript><p></noscript><img src=x onerror=alert(1)>'),
			'<img>'
		);
	});

	it('drops template wrappers instead of leaking their contents', () => {
		// <template> contents live in a separate fragment, so unwrapping it hands
		// the inner img back as ordinary markup that then gets sanitized.
		assert.equal(sanitizeGuideHtml('<template><img src=x onerror=alert(1)>'), '<img>');
		assert.equal(
			sanitizeGuideHtml('<template></template><img src=x onerror=alert(1)>'),
			'<img>'
		);
	});

	it('strips javascript: hrefs from anchors nested inside allowed tags', () => {
		assert.equal(
			sanitizeGuideHtml('<p><a href="javascript:alert(1)"><strong>x</strong></a></p>'),
			'<p><a><strong>x</strong></a></p>'
		);
		assert.equal(
			sanitizeGuideHtml('<ul><li><a href="JaVaScRiPt:alert(1)"><em>y</em></a></li></ul>'),
			'<ul><li><a><em>y</em></a></li></ul>'
		);
		assert.equal(
			sanitizeGuideHtml('<td><a href="  java\tscript:alert(1)">x</a></td>'),
			'<td><a>x</a></td>'
		);
	});

	it('emits no MathML or foreign-content markup', () => {
		// math and mi are both outside the allowed set, so the element is
		// unwrapped whole and neither the xlink:href nor the <script> inside the
		// data: URL survives.
		assert.equal(
			sanitizeGuideHtml('<math><mi//xlink:href="data:x,<script>alert(1)</script>">'),
			''
		);
		assert.equal(
			sanitizeGuideHtml('<math><mtext><img src=x onerror=alert(1)></mtext></math>'),
			'<img>'
		);
	});

	it('handles deeply nested unclosed inline tags', () => {
		assert.equal(sanitizeGuideHtml('<b><i><u>text'), '<b><i>text');
		const deep = '<b><i><u>'.repeat(64) + 'text';
		assert.equal(sanitizeGuideHtml(deep), '<b><i>'.repeat(64) + 'text');
	});

	it('recognizes an uppercase http scheme', () => {
		assert.equal(
			sanitizeGuideHtml('<a href="HTTPS://evil.com/x">x</a>'),
			'<a href="HTTPS://evil.com/x">x</a>'
		);
		assert.equal(
			sanitizeGuideHtml('<a href="hTtPs://evil.com/x">x</a>'),
			'<a href="hTtPs://evil.com/x">x</a>'
		);
		assert.equal(sanitizeGuideHtml('<a href="HTTPS:evil.com">x</a>'), '<a>x</a>');
		assert.equal(
			sanitizeGuideHtml('<img SRC="HTTPS://evil.com/x.png" ALT="p">'),
			'<img src="HTTPS://evil.com/x.png" alt="p">'
		);
	});

	it('escapes an unterminated tag so trailing markup cannot complete it', () => {
		// With no trailing '>' this tokenizer sees plain text, but a browser
		// keeps reading into the surrounding document and closes the element on
		// the next '>', reviving the handler.
		assert.equal(
			sanitizeGuideHtml('<p>hi</p><img src=x onerror=alert(1)'),
			'<p>hi</p>&lt;img src=x onerror=alert(1)'
		);
		assert.equal(sanitizeGuideHtml('<p>hi</p><script'), '<p>hi</p>&lt;script');
		assert.equal(sanitizeGuideHtml('<p>hi</p><svg onload=alert(1)'), '<p>hi</p>&lt;svg onload=alert(1)');
		assert.equal(
			sanitizeGuideHtml('<p>hi</p><a href=javascript:alert(1)'),
			'<p>hi</p>&lt;a href=javascript:alert(1)'
		);
		assert.equal(sanitizeGuideHtml('a <! b'), 'a &lt;! b');
	});
});
