import { marked, type Token, type Tokens } from 'marked';
import { highlightAura, highlightCode } from '$lib/aura/highlight';

export interface TocEntry {
	id: string;
	text: string;
	level: number;
}

function escHtml(s: string): string {
	return s
		.replace(/&/g, '&amp;')
		.replace(/</g, '&lt;')
		.replace(/>/g, '&gt;')
		.replace(/"/g, '&quot;');
}

function stripTags(html: string): string {
	return html
		.replace(/<[^>]*>/g, '')
		.replace(/&amp;/g, '&')
		.replace(/&lt;/g, '<')
		.replace(/&gt;/g, '>')
		.replace(/&quot;/g, '"');
}

function slugify(text: string): string {
	return text
		.toLowerCase()
		.replace(/[^a-z0-9\s-]/g, '')
		.trim()
		.replace(/\s+/g, '-');
}

// Verbatim path data from the lucide.dev `external-link` icon, rendered as
// a string because the markdown renderer outputs raw HTML.
const LINK_ICON =
	`<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" ` +
	`stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">` +
	`<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/>` +
	`<path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg>`;

const EXTERNAL_ICON =
	`<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" ` +
	`stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">` +
	`<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/></svg>`;

function renderCode(raw: string, lang: string): string {
	const code = raw ?? '';
	const runnable = lang === 'rnx' ? ' data-runnable="rnx"' : '';
	const copy = `<button data-copy data-code="${escHtml(code)}" class="doc-btn">copy</button>`;
	const actions =
		lang === 'rnx'
			? `<button data-playground data-code="${escHtml(code)}" class="doc-btn doc-run" title="Open in Playground">${EXTERNAL_ICON}<span>open</span></button>${copy}`
			: copy;
	const body =
		lang === 'rnx'
			? `<pre><code>${highlightAura(code)
					.map((s) => `<span class="${s.cls}">${escHtml(s.text)}</span>`)
					.join('')}</code></pre>`
			: (() => {
					const generic = highlightCode(code, lang);
					if (generic) {
						return `<pre><code>${generic
							.map((s) => `<span class="${s.cls}">${escHtml(s.text)}</span>`)
							.join('')}</code></pre>`;
					}
					return `<pre><code>${escHtml(code)}</code></pre>`;
				})();
	return (
		`<div class="doc-code"${runnable} data-code="${escHtml(code)}" data-lang="${lang || 'text'}">` +
		`<div class="doc-codehead"><span class="doc-lang">${lang === 'rnx' ? '.rnx' : escHtml(lang || 'text')}</span>` +
		`<span class="doc-actions">${actions}</span></div>${body}</div>`
	);
}

export interface DocMeta {
	title: string;
	description: string;
	track?: string;
	sourceLang?: string;
}

export function parseFrontmatter(src: string): { meta: DocMeta; body: string } {
	const meta: DocMeta = { title: '', description: '' };
	if (!src.startsWith('---\n')) return { meta, body: src };
	const end = src.indexOf('\n---', 4);
	if (end < 0) return { meta, body: src };
	for (const line of src.slice(4, end).split('\n')) {
		const colon = line.indexOf(':');
		if (colon < 0) continue;
		const key = line.slice(0, colon).trim();
		const value = line
			.slice(colon + 1)
			.trim()
			.replace(/^"(.*)"$/, '$1');
		if (key === 'title' || key === 'description' || key === 'track' || key === 'sourceLang') {
			(meta as unknown as Record<string, string>)[key] = value;
		}
	}
	return { meta, body: src.slice(end + 4).replace(/^\n/, '') };
}

export interface CompareBlock {
	lang: string;
	other: string;
	rnx: string;
}

export interface DocSection {
	html: string;
	compare?: CompareBlock;
}

const ROSETTA_LANGS: Record<string, string> = {
	rust: 'Rust',
	go: 'Go',
	cpp: 'C++',
	c: 'C',
	typescript: 'TypeScript'
};

export function splitSections(src: string): { sections: DocSection[]; toc: TocEntry[] } {
	const { body } = parseFrontmatter(src);
	const tokens = marked.lexer(body);
	while (tokens.length > 0 && tokens[0].type === 'space') {
		tokens.shift();
	}
	if (tokens.length > 0 && tokens[0].type === 'heading' && tokens[0].depth === 1) {
		tokens.shift();
	}
	const toc: TocEntry[] = [];
	for (const t of tokens) {
		if (t.type === 'heading' && (t.depth === 2 || t.depth === 3)) {
			const inline = marked.parseInline(t.text) as string;
			toc.push({
				id: slugify(stripTags(t.text)),
				text: stripTags(inline),
				level: t.depth
			});
		}
	}
	const groups: Token[][] = [[]];
	for (const t of tokens) {
		if (t.type === 'heading' && t.depth === 2) groups.push([t]);
		else groups[groups.length - 1].push(t);
	}
	const sections: DocSection[] = [];
	for (const g of groups) {
		if (g.length === 0) continue;
		const codes = g.filter((t) => t.type === 'code') as Tokens.Code[];
		const rnx = codes.find((c) => (c.lang ?? '').toLowerCase() === 'rnx');
		const other = codes.find((c) => ROSETTA_LANGS[(c.lang ?? '').toLowerCase()] !== undefined);
		if (rnx && other) {
			const rest = g.filter((t) => t !== rnx && t !== other);
			sections.push({
				html: marked.parser(rest) as string,
				compare: {
					lang: ROSETTA_LANGS[other.lang!.toLowerCase()],
					other: other.text,
					rnx: rnx.text
				}
			});
		} else {
			sections.push({ html: marked.parser(g) as string });
		}
	}
	return { sections, toc };
}

export interface RawSection {
	id: string;
	title: string;
	markdown: string;
}

// Splits a markdown body into its `##` subtopics, each carrying the raw
// markdown of that section including its own `##` heading line. Ids are
// computed exactly like the rendered `<h2 id>` values so clients can
// match buttons to headings. Content before the first `##` (title,
// preamble) belongs to no section.
export function splitRawSections(src: string): RawSection[] {
	const { body } = parseFrontmatter(src);
	const tokens = marked.lexer(body);
	while (tokens.length > 0 && tokens[0].type === 'space') {
		tokens.shift();
	}
	if (tokens.length > 0 && tokens[0].type === 'heading' && tokens[0].depth === 1) {
		tokens.shift();
	}
	const sections: RawSection[] = [];
	let current: { title: string; id: string; raws: string[] } | null = null;
	const rawOf = (t: Token): string => (t as { raw?: string }).raw ?? '';
	for (const t of tokens) {
		if (t.type === 'heading' && t.depth === 2) {
			if (current) {
				sections.push({
					id: current.id,
					title: current.title,
					markdown: current.raws.join('').trim() + '\n'
				});
			}
			const inline = marked.parseInline(t.text) as string;
			current = {
				title: stripTags(inline),
				id: slugify(stripTags(t.text)),
				raws: [rawOf(t)]
			};
		} else if (current) {
			current.raws.push(rawOf(t));
		}
	}
	if (current) {
		sections.push({
			id: current.id,
			title: current.title,
			markdown: current.raws.join('').trim() + '\n'
		});
	}
	return sections;
}

let tocSink: TocEntry[] | null = null;

// Inline SVGs below reuse verbatim path data from lucide.dev icons
// (info, lightbulb, circle-alert, triangle-alert, octagon-alert),
// rendered as strings because the markdown renderer outputs raw HTML.
function admonitionIcon(kind: string): string {
	const paths: Record<string, string> = {
		note: '<circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/>',
		tip: '<path d="M15 14c.2-1 .7-1.7 1.5-2.5 1-.9 1.5-2.2 1.5-3.5A6 6 0 0 0 6 8c0 1 .2 2.2 1.5 3.5.7.7 1.3 1.5 1.5 2.5"/><path d="M9 18h6"/><path d="M10 22h4"/>',
		important:
			'<circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/>',
		warning:
			'<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><path d="M12 9v4"/><path d="M12 17h.01"/>',
		caution:
			'<polygon points="7.86 2 16.14 2 22 7.86 22 16.14 16.14 22 7.86 22 2 16.14 2 7.86 7.86 2"/><path d="M12 8v4"/><path d="M12 16h.01"/>'
	};
	const label: Record<string, string> = {
		note: 'Note',
		tip: 'Tip',
		important: 'Important',
		warning: 'Warning',
		caution: 'Caution'
	};
	return (
		`<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" ` +
		`stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[kind]}</svg>` +
		`<span>${label[kind]}</span>`
	);
}

const renderer = {
	heading({ text, depth }: Tokens.Heading) {
		const inline = marked.parseInline(text) as string;
		const id = slugify(stripTags(text));
		const plain = stripTags(inline);
		if (tocSink && (depth === 2 || depth === 3)) tocSink.push({ id, text: plain, level: depth });
		return `<h${depth} id="${id}" class="doc-h">${inline}<a href="#${id}" class="doc-anchor" aria-label=".Anchor"> ${LINK_ICON}</a></h${depth}>`;
	},
	code({ text, lang }: Tokens.Code) {
		return renderCode(text, (lang ?? '').toLowerCase());
	},
	blockquote({ tokens }: Tokens.Blockquote) {
		const inner = marked.parser(tokens) as string;
		const m = inner.match(/^<p>\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*/);
		if (!m) return `<blockquote>${inner}</blockquote>`;
		const kind = m[1].toLowerCase();
		return `<aside class="callout callout-${kind}"><p class="callout-label">${admonitionIcon(kind)}</p>${inner.slice(m[0].length)}</aside>`;
	},
	table({ header, rows }: Tokens.Table) {
		const head = `<tr>${header.map((c) => `<th>${marked.parseInline(c.text)}</th>`).join('')}</tr>`;
		const body = rows.map(
			(r) => `<tr>${r.map((c) => `<td>${marked.parseInline(c.text)}</td>`).join('')}</tr>`
		);
		return `<div class="doc-table-wrap"><table class="doc-table"><thead>${head}</thead><tbody>${body.join('')}</tbody></table></div>`;
	}
};

marked.use({ renderer });

export function renderMarkdown(src: string): { html: string; toc: TocEntry[] } {
	const { body } = parseFrontmatter(src);
	const toc: TocEntry[] = [];
	tocSink = toc;
	try {
		const html = marked.parse(body, { async: false }) as string;
		return { html, toc };
	} finally {
		tocSink = null;
	}
}
