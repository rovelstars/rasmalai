import { before, describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Language, Parser, Query } from 'web-tree-sitter';
import { captureClass, spansFromCaptures, type AuraSpan } from '../src/lib/aura/rnx-captures.ts';

const root = join(import.meta.dirname, '..');

describe('rnx capture classes', () => {
	it('maps the query captures to the Aura roles', () => {
		assert.equal(captureClass('keyword'), 'text-aura-purple');
		assert.equal(captureClass('keyword.control.import'), 'text-aura-purple');
		assert.equal(captureClass('type'), 'text-aura-pink');
		assert.equal(captureClass('type.builtin'), 'text-aura-pink');
		assert.equal(captureClass('function.call'), 'text-aura-cyan');
		assert.equal(captureClass('property'), 'text-aura-cyan');
		assert.equal(captureClass('string'), 'text-aura-orange');
		assert.equal(captureClass('comment.documentation'), 'text-aura-green');
		assert.equal(captureClass('comment'), 'text-aura-muted italic');
		assert.equal(captureClass('unknown'), 'text-aura-text');
	});

	it('coalesces adjacent spans and lets later captures win', () => {
		const spans = spansFromCaptures('fn Int', [
			{ name: 'keyword', start: 0, end: 2 },
			{ name: 'type', start: 3, end: 6 }
		]);
		assert.deepEqual(spans, [
			{ text: 'fn', cls: 'text-aura-purple' },
			{ text: ' ', cls: 'text-aura-text' },
			{ text: 'Int', cls: 'text-aura-pink' }
		]);
		const overridden = spansFromCaptures('Page', [
			{ name: 'function.call', start: 0, end: 4 },
			{ name: 'type', start: 0, end: 4 }
		]);
		assert.deepEqual(overridden, [{ text: 'Page', cls: 'text-aura-pink' }]);
	});
});

describe('rnx tree-sitter query', () => {
	let parser: Parser;
	let query: Query;

	before(async () => {
		await Parser.init({
			locateFile: (name: string) => join(root, 'node_modules', 'web-tree-sitter', name)
		});
		const language = await Language.load(join(root, 'static', 'rnx.wasm'));
		parser = new Parser();
		parser.setLanguage(language);
		query = new Query(
			language,
			readFileSync(join(root, '..', 'editors', 'tree-sitter-rasmalai', 'queries', 'highlights.scm'), 'utf8')
		);
	});

	function toSpans(src: string, links?: Map<string, string>): AuraSpan[] {
		const tree = parser.parse(src);
		assert.ok(tree);
		const captures = query.captures(tree.rootNode).map((capture) => ({
			name: capture.name,
			start: capture.node.startIndex,
			end: capture.node.endIndex
		}));
		return spansFromCaptures(src, captures, links);
	}

	function clsOf(spans: AuraSpan[], text: string): string | undefined {
		return spans.find((span) => span.text === text)?.cls;
	}

	it('classifies imports, capitalized calls, and named arguments', () => {
		const source =
			'import { App } from "@std/app"\nclass Tasks { let done: Bool = false }\nfn Main(): Int { return Page(title: "x") }';
		const spans = toSpans(source);
		assert.equal(clsOf(spans, 'import'), 'text-aura-purple');
		assert.equal(clsOf(spans, 'App'), 'text-aura-pink');
		assert.equal(clsOf(spans, 'Tasks'), 'text-aura-pink');
		assert.equal(clsOf(spans, 'Bool'), 'text-aura-pink');
		assert.equal(clsOf(spans, 'Main'), 'text-aura-cyan');
		assert.equal(clsOf(spans, 'Page'), 'text-aura-pink');
		assert.equal(clsOf(spans, 'title'), 'text-aura-cyan');
		assert.equal(clsOf(spans, '"x"'), 'text-aura-orange');
	});

	it('highlights member calls inside a call trailing block', () => {
		const source =
			'fn screen(state: Tasks): View {\n    return Page(title: "Today") {\n        Header {\n            Button("Clear", () => state.clear())\n        }\n    }\n}';
		const spans = toSpans(source);
		assert.equal(clsOf(spans, 'clear'), 'text-aura-cyan');
		assert.equal(clsOf(spans, 'state'), 'text-aura-text');
		assert.equal(clsOf(spans, 'Button'), 'text-aura-pink');
	});

	it('links type spans when a link map provides one', () => {
		const spans = toSpans('fn Main(): View { return Page() }', new Map([['View', '/std/ui#View']]));
		const view = spans.find((span) => span.text === 'View');
		assert.equal(view?.cls, 'text-aura-pink');
		assert.equal(view?.href, '/std/ui#View');
	});
});
