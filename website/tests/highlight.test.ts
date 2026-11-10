import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { highlightAura, highlightInlineCode } from '../src/lib/aura/highlight.js';

describe('highlightAura doc comments', () => {
	it('paints /** */ blocks differently from plain comments', () => {
		const doc = highlightAura('/** Adds one. */');
		const plain = highlightAura('/* Adds one. */');
		assert.ok(doc.some((s) => s.cls === 'text-aura-green'), JSON.stringify(doc));
		assert.ok(
			plain.every((s) => s.cls === 'text-aura-muted' || s.cls === 'text-aura-muted italic'),
			JSON.stringify(plain)
		);
		assert.notEqual(
			doc.map((s) => s.cls).join(','),
			plain.map((s) => s.cls).join(',')
		);
	});

	it('paints //! lines as docs and /// lines as plain comments', () => {
		const module = highlightAura('//! Module docs.');
		assert.ok(module.some((s) => s.cls === 'text-aura-green'), JSON.stringify(module));
		const triple = highlightAura('/// Subtracts one.');
		assert.ok(
			triple.every((s) => s.cls === 'text-aura-muted italic'),
			JSON.stringify(triple)
		);
	});

	it('leaves /**/ as a plain comment', () => {
		const spans = highlightAura('/**/');
		assert.ok(
			spans.every((s) => s.cls === 'text-aura-muted italic'),
			JSON.stringify(spans)
		);
	});

	it('paints @param inside doc blocks with the tag class', () => {
		const spans = highlightAura('/** @param x the value */');
		assert.ok(
			spans.some((s) => s.text === '@param' && s.cls === 'text-aura-cyan'),
			JSON.stringify(spans)
		);
	});
});

describe('highlightAura oracle spot checks', () => {
	it('paints oracle keywords purple (token.rs)', () => {
		const spans = highlightAura('fn export pass new interface extension super taken');
		const clsOf = (word: string) => spans.find((s) => s.text.includes(word))?.cls;
		for (const word of ['fn', 'export', 'pass', 'new', 'interface', 'extension', 'super']) {
			assert.equal(clsOf(word), 'text-aura-purple', word);
		}
		assert.equal(clsOf('taken'), 'text-aura-text');
	});

	it('paints oracle types pink (highlight.rs is_type_name)', () => {
		const spans = highlightAura('let o: Option = 0;');
		assert.ok(
			spans.some((s) => s.text === 'Option' && s.cls === 'text-aura-pink'),
			JSON.stringify(spans)
		);
	});
});

describe('highlightAura newer syntax', () => {
	it('paints macros, attributes, function names, and members', () => {
		const spans = highlightAura(
			'#[Macro]\nfn run(): Int {\n    println!("ok");\n    return this.value;\n}'
		);
		const clsOf = (word: string) => spans.find((s) => s.text === word)?.cls;
		assert.equal(clsOf('Macro'), 'text-aura-cyan');
		assert.equal(clsOf('run'), 'text-aura-cyan');
		assert.equal(clsOf('println'), 'text-aura-cyan');
		assert.equal(clsOf('this'), 'text-aura-purple');
		assert.equal(clsOf('value'), 'text-aura-cyan');
	});

	it('paints enum variants, named arguments, and multi-character operators', () => {
		const spans = highlightAura('let p = Page(title: "x"); case .Circle(r): return a ?? b;');
		const clsOf = (word: string) => spans.find((s) => s.text === word)?.cls;
		assert.equal(clsOf('Page'), 'text-aura-pink');
		assert.equal(clsOf('title'), 'text-aura-cyan');
		assert.equal(clsOf('Circle'), 'text-aura-pink');
		assert.ok(spans.some((s) => s.text === '??' && s.cls === 'text-aura-purple'), JSON.stringify(spans));
	});

	it('paints runtime shims as keywords', () => {
		const spans = highlightAura('import { fn len(): Int } from runtime "core";');
		assert.equal(spans.find((s) => s.text === 'runtime')?.cls, 'text-aura-purple');
	});

	it('highlights interpolated expressions inside strings', () => {
		const spans = highlightAura('println!("n=${count + 1}")');
		assert.ok(spans.some((s) => s.text === 'count' && s.cls === 'text-aura-text'), JSON.stringify(spans));
	});
});

describe('highlightInlineCode', () => {
	it('paints known tokens without reclassifying plain inline text', () => {
		const spans = highlightInlineCode('Motion.ofCurve(curve, duration)');
		const clsOf = (word: string) => spans.find((s) => s.text.includes(word))?.cls;
		assert.equal(clsOf('Motion'), 'text-aura-text');
		assert.equal(clsOf('ofCurve'), 'text-aura-cyan');
		assert.equal(clsOf('curve'), 'text-aura-text');
		assert.equal(clsOf('duration'), 'text-aura-text');
	});

	it('leaves error codes, paths, and flags plain', () => {
		for (const text of ['E108', 'Project.config', '--target', 'MIT OR Apache-2.0']) {
			const spans = highlightInlineCode(text);
			assert.ok(
				spans.every((s) => !s.cls.includes('pink') && !s.cls.includes('muted')),
				`${text}: ${JSON.stringify(spans)}`
			);
		}
	});

	it('paints types, keywords, macros, and literals', () => {
		const spans = highlightInlineCode('Int println! true 42');
		assert.equal(spans.find((s) => s.text === 'Int')?.cls, 'text-aura-pink');
		assert.equal(spans.find((s) => s.text === 'println')?.cls, 'text-aura-cyan');
		assert.equal(spans.find((s) => s.text === 'true')?.cls, 'text-aura-orange');
		assert.equal(spans.find((s) => s.text === '42')?.cls, 'text-aura-orange');
	});
});
