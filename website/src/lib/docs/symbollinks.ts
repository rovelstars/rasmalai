import type { DocModule } from '$lib/docs/api';

export type SymbolKind = 'class' | 'enum' | 'fn' | 'const';

export interface SymbolEntry {
	name: string;
	kind: SymbolKind;
	module: string;
	anchor: string;
}

export function anchorFor(kind: SymbolKind, name: string): string {
	return `${kind}-${name}`;
}

export function defaultStdHref(moduleName: string, anchor: string): string {
	return `/docs/@std/${moduleName}/api#${anchor}`;
}

export function buildSymbolIndex(
	modules: DocModule[],
	hrefFor: (moduleName: string, anchor: string) => string = defaultStdHref
): Map<string, string> {
	const index = new Map<string, string>();
	for (const m of modules) {
		const add = (kind: SymbolKind, name: string) => {
			if (!name || index.has(name)) return;
			index.set(name, hrefFor(m.name, anchorFor(kind, name)));
		};
		for (const c of m.classes ?? []) add('class', c.name);
		for (const e of m.enums ?? []) add('enum', e.name);
		for (const f of m.functions ?? []) add('fn', f.name);
		for (const c of m.constants ?? []) add('const', c.name);
	}
	return index;
}

function escReg(s: string): string {
	return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function matchNames(
	index: Map<string, string>,
	exclude?: Set<string>
): { re: RegExp | null; hrefOf: (m: string) => string | undefined } {
	const names = [...index.keys()].filter((n) => /^[A-Z]/.test(n) && !(exclude && exclude.has(n)));
	if (names.length === 0) return { re: null, hrefOf: () => undefined };
	const pluralOf = new Map<string, string>();
	for (const n of names) {
		if (!n.endsWith('s') && !index.has(n + 's')) pluralOf.set(n + 's', n);
	}
	const alts = [...names, ...pluralOf.keys()];
	alts.sort((a, b) => b.length - a.length);
	const re = new RegExp(`(?<![A-Za-z0-9_])(${alts.map(escReg).join('|')})(?![A-Za-z0-9_])`, 'g');
	return {
		re,
		hrefOf: (m: string) => {
			const exact = index.get(m);
			if (exact) return exact;
			const singular = pluralOf.get(m);
			return singular ? index.get(singular) : undefined;
		}
	};
}

function splitHtml(html: string): { tag: boolean; text: string }[] {
	const parts = html.split(/(<[^>]*>)/g);
	return parts.filter((p) => p.length > 0).map((p) => ({ tag: p.startsWith('<'), text: p }));
}

function escHtmlText(s: string): string {
	return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

export function linkifyTypeText(ty: string, index: Map<string, string>): string {
	const { re, hrefOf } = matchNames(index);
	if (!re) return escHtmlText(ty);
	return escHtmlText(ty).replace(re, (m) => `<a href="${hrefOf(m)}" class="doc-symlink">${m}</a>`);
}

function isOpenTag(token: string, name: string): boolean {
	return new RegExp(`^<${name}[\\s>]`, 'i').test(token);
}

function isCloseTag(token: string, name: string): boolean {
	return new RegExp(`^</${name}\\s*>`, 'i').test(token);
}

export function linkifyProseHtml(
	html: string,
	index: Map<string, string>,
	exclude?: Set<string>
): string {
	const { re, hrefOf } = matchNames(index, exclude);
	if (!re) return html;
	let codeDepth = 0;
	let out = '';
	for (const part of splitHtml(html)) {
		if (part.tag) {
			if (isOpenTag(part.text, 'code') || isOpenTag(part.text, 'pre')) codeDepth++;
			if (isCloseTag(part.text, 'code') || isCloseTag(part.text, 'pre')) codeDepth = Math.max(0, codeDepth - 1);
			out += part.text;
			continue;
		}
		if (codeDepth > 0) {
			out += part.text;
			continue;
		}
		out += part.text.replace(re, (m) => `<a href="${hrefOf(m)}" class="doc-symlink">${m}</a>`);
	}
	return out;
}
