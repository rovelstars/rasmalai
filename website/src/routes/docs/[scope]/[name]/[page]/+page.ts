import { error } from '@sveltejs/kit';
import { renderMarkdown, splitRawSections } from '$lib/docs/markdown';
import {
	genericSnippet,
	gettingStartedMarkdown,
	moduleFor,
	overviewMarkdown
} from '$lib/docs/module-doc';
import { importSnippet, quickstartFor } from '$lib/docs/stdlib';
import type { DocModule } from '$lib/docs/api';

const PAGES = ['overview', 'getting-started', 'api'] as const;

export function entries() {
	const out: { scope: string; name: string; page: string }[] = [];
	const mods = [
		'prelude',
		'simd',
		'collections',
		'math',
		'fs',
		'bytes',
		'time',
		'random',
		'sync',
		'env',
		'process',
		'os',
		'testing',
		'web',
		'net',
		'json',
		'io'
	];
	for (const name of mods) {
		for (const page of PAGES) out.push({ scope: '@std', name, page });
	}
	return out;
}

interface ServerData {
	full: string;
	scope: string;
	name: string;
	page: string;
	version: string;
	description: string;
	mods: DocModule[];
	guides: Array<{ slug: string; title: string; html: string; source: string }>;
}

function stdStarted(full: string, mod: DocModule | null): { importLine: string; snippet: string } {
	const short = full.replace(/^@std\//, '');
	let snippet: string;
	try {
		snippet = quickstartFor(short);
	} catch {
		snippet = genericSnippet(full, mod);
	}
	let importLine: string;
	try {
		importLine = importSnippet(short);
	} catch {
		importLine = genericSnippet(full, mod);
	}
	return { importLine, snippet };
}

function registryStarted(full: string, mod: DocModule | null): { importLine: string; snippet: string } {
	const line = genericSnippet(full, mod);
	if (line.startsWith('//')) return { importLine: line, snippet: line };
	return {
		importLine: line,
		snippet: `${line}\n// exact call shapes live in the API reference below`
	};
}

export async function load({ params, data }: { params: Record<string, string>; data: ServerData }) {
	if (!PAGES.includes(params.page as (typeof PAGES)[number])) error(404, 'doc page not found');
	const full = data.full;
	const isStd = full.startsWith('@std/');
	const mod = moduleFor(data.mods, params.name);
	if (params.page === 'api') {
		const toc = mod
			? [
					...mod.classes.map((c) => ({ id: `class-${c.name}`, text: c.name, level: 2 })),
					...mod.enums.map((e) => ({ id: `enum-${e.name}`, text: e.name, level: 2 })),
					...mod.functions.map((f) => ({ id: `fn-${f.name}`, text: `${f.name}()`, level: 2 }))
				]
			: [];
		return {
			full,
			scope: data.scope,
			module: params.name,
			page: params.page,
			version: data.version,
			description: data.description,
			html: null as string | null,
			toc,
			mod,
			guides: data.guides,
			noApi: mod === null
		};
	}
	if (!mod) {
		return {
			full,
			scope: data.scope,
			module: params.name,
			page: params.page,
			version: data.version,
			description: data.description,
			html: null as string | null,
			toc: [],
			raw: '',
			sections: [],
			mod,
			guides: data.guides,
			fromGuide: null as string | null,
			noApi: true
		};
	}
	const started = isStd ? stdStarted(full, mod) : registryStarted(full, mod);
	const src =
		params.page === 'overview'
			? overviewMarkdown(full, mod, data.description)
			: gettingStartedMarkdown(full, {
					importLine: started.importLine,
					snippet: started.snippet,
					installed: isStd
				});
	const stored = data.guides.find((g) => g.slug === params.page);
	if (stored && stored.html) {
		const rendered = renderMarkdown(stored.source.replace(/^\s*# .*\n/, ''));
		return {
			full,
			scope: data.scope,
			module: params.name,
			page: params.page,
			version: data.version,
			description: data.description,
			html: stored.html,
			toc: rendered.toc,
			raw: stored.source,
			sections: splitRawSections(stored.source),
			mod,
			guides: data.guides,
			fromGuide: stored.slug,
			noApi: false
		};
	}
	const { html, toc } = renderMarkdown(src.replace(/^\s*# .*\n/, ''));
	return {
		full,
		scope: data.scope,
		module: params.name,
		page: params.page,
		version: data.version,
		description: data.description,
		html: html as string | null,
		toc,
		raw: src,
		sections: splitRawSections(src),
		mod,
		guides: data.guides,
		fromGuide: null as string | null,
		noApi: false
	};
}
