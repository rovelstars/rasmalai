import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { highlightAura } from '../src/lib/aura/highlight.js';

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
		const clsOf = (word: string) => spans.find((s) => s.text === word)?.cls;
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
