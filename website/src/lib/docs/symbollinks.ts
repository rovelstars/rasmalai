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

function splitHtml(html: string): { tag: boolean; text: string }[] {
	const parts = html.split(/(<[^>]*>)/g);
	return parts.filter((p) => p.length > 0).map((p) => ({ tag: p.startsWith('<'), text: p }));
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
	const names = [...index.keys()].filter((n) => /^[A-Z]/.test(n) && !(exclude && exclude.has(n)));
	if (names.length === 0) return html;
	names.sort((a, b) => b.length - a.length);
	const re = new RegExp(`(?<![A-Za-z0-9_])(${names.map(escReg).join('|')})(?![A-Za-z0-9_])`, 'g');
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
		out += part.text.replace(re, (m) => `<a href="${index.get(m)}" class="symlink">${m}</a>`);
	}
	return out;
}
