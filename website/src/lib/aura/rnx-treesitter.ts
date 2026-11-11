import type { Extension } from '@codemirror/state';
import { Decoration, EditorView, ViewPlugin, type DecorationSet, type ViewUpdate } from '@codemirror/view';
import { base } from '$app/paths';
import {
	escapeRnxHtml,
	spansFromCaptures,
	type AuraSpan
} from './rnx-captures';
import { RNX_BLOCK_SPANS } from './rnx-highlight.gen';
import { RNX_HIGHLIGHTS } from './rnx-query.gen';

interface Highlighter {
	spans(src: string, links?: Map<string, string>): AuraSpan[];
}

let pending: Promise<Highlighter | null> | null = null;

async function load(): Promise<Highlighter | null> {
	try {
		const { Parser, Language, Query } = await import('web-tree-sitter');
		await Parser.init({ locateFile: (name: string) => `${base}/${name}` });
		const response = await fetch(`${base}/rnx.wasm`);
		if (!response.ok) return null;
		const language = await Language.load(new Uint8Array(await response.arrayBuffer()));
		const parser = new Parser();
		parser.setLanguage(language);
		const query = new Query(language, RNX_HIGHLIGHTS);
		return {
			spans(src, links) {
				const tree = parser.parse(src);
				if (!tree) return [];
				const captures = query.captures(tree.rootNode).map((capture) => ({
					name: capture.name,
					start: capture.node.startIndex,
					end: capture.node.endIndex
				}));
				return spansFromCaptures(src, captures, links);
			}
		};
	} catch {
		return null;
	}
}

export function initRnxHighlighter(): Promise<Highlighter | null> {
	if (!pending) pending = load();
	return pending;
}

export function plainRnxSpans(src: string): AuraSpan[] {
	return [{ text: src, cls: 'text-aura-text' }];
}

export function cachedRnxSpans(src: string, links?: Map<string, string>): AuraSpan[] {
	const spans = RNX_BLOCK_SPANS[src];
	if (!spans) return plainRnxSpans(src);
	if (!links) return spans;
	return spans.map((span) =>
		span.cls === 'text-aura-pink' && links.has(span.text) ? { ...span, href: links.get(span.text) } : span
	);
}

export async function highlightRnxSpans(src: string, links?: Map<string, string>): Promise<AuraSpan[]> {
	const highlighter = await initRnxHighlighter();
	if (!highlighter) return cachedRnxSpans(src, links);
	const spans = highlighter.spans(src, links);
	return spans.length > 0 ? spans : plainRnxSpans(src);
}

export async function highlightRnxLines(lines: readonly string[]): Promise<string[]> {
	const highlighter = await initRnxHighlighter();
	if (!highlighter) return lines.map((line) => `<span class="text-aura-text">${escapeRnxHtml(line)}</span>`);
	const src = lines.join('\n');
	const spans = highlighter.spans(src);
	const out: string[] = [];
	let html = '';
	for (const span of spans) {
		const pieces = span.text.split('\n');
		for (let i = 0; i < pieces.length; i += 1) {
			if (i > 0) {
				out.push(html);
				html = '';
			}
			if (pieces[i]) html += `<span class="${span.cls}">${escapeRnxHtml(pieces[i])}</span>`;
		}
	}
	out.push(html);
	return out;
}

class RnxHighlightPlugin {
	decorations: DecorationSet = Decoration.none;
	private destroyed = false;

	constructor(view: EditorView) {
		this.rebuild(view);
	}

	update(update: ViewUpdate): void {
		if (update.docChanged || update.viewportChanged) this.rebuild(update.view);
	}

	destroy(): void {
		this.destroyed = true;
	}

	private rebuild(view: EditorView): void {
		initRnxHighlighter().then((highlighter) => {
			if (this.destroyed || !highlighter) return;
			const spans = highlighter.spans(view.state.doc.toString());
			const ranges = [];
			let offset = 0;
			for (const span of spans) {
				const end = offset + span.text.length;
				if (span.cls !== 'text-aura-text' && end > offset) {
					ranges.push(Decoration.mark({ class: span.cls }).range(offset, end));
				}
				offset = end;
			}
			this.decorations = Decoration.set(ranges, true);
			view.dispatch({});
		});
	}
}

export function rnxTreeSitter(): Extension {
	return ViewPlugin.fromClass(RnxHighlightPlugin, {
		decorations: (plugin) => plugin.decorations
	});
}
