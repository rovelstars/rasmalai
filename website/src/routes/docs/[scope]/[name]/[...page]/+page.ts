import { error } from '@sveltejs/kit';
import { renderMarkdown, splitRawSections } from '$lib/docs/markdown';
import {
	genericSnippet,
	gettingStartedMarkdown,
	moduleFor,
	overviewMarkdown
} from '$lib/docs/module-doc';
import { importSnippet, quickstartFor } from '$lib/docs/stdlib';
import { buildPackagePageNav } from '$lib/docs/package-pages';
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
		'io',
		'ui',
		'app'
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
	versions: string[];
	latest: string;
	description: string;
	mods: DocModule[];
	guides: Array<{
		slug: string;
		title: string;
		html: string;
		source: string;
		section?: string | null;
		order?: string | null;
	}>;
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
	const page = params.page ?? '';
	const full = data.full;
	const isStd = full.startsWith('@std/');
	const mod = moduleFor(data.mods, params.name);
	const stored = data.guides.find((g) => g.slug === page);
	const isBase = PAGES.includes(page as (typeof PAGES)[number]);
	if (!isBase && !stored) error(404, 'doc page not found');
	const title = isBase
		? page === 'overview'
			? 'Overview'
			: page === 'getting-started'
				? 'Getting Started'
				: 'API Reference'
		: stored?.title ?? page;
	const nav = buildPackagePageNav(data.guides);
	const index = nav.order.findIndex((entry) => entry.slug === page);
	const pager = {
		prev: index > 0 ? { path: `/docs/${full}/${nav.order[index - 1].slug}`, title: nav.order[index - 1].title } : null,
		next:
			index >= 0 && index < nav.order.length - 1
				? { path: `/docs/${full}/${nav.order[index + 1].slug}`, title: nav.order[index + 1].title }
				: null
	};

	if (page === 'api') {
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
			page,
			title,
			version: data.version,
			versions: data.versions,
			latest: data.latest,
			description: data.description,
			html: null as string | null,
			toc,
			mod,
			guides: data.guides,
			...pager,
			noApi: mod === null
		};
	}
	if (stored) {
		const rendered = renderMarkdown(stored.source.replace(/^\s*# .*\n/, ''));
		return {
			full,
			scope: data.scope,
			module: params.name,
			page,
			title,
			version: data.version,
			versions: data.versions,
			latest: data.latest,
			description: data.description,
			html: rendered.html,
			toc: rendered.toc,
			raw: stored.source,
			sections: splitRawSections(stored.source),
			mod,
			guides: data.guides,
			...pager,
			fromGuide: stored.slug,
			noApi: false
		};
	}
	if (!mod) {
		return {
			full,
			scope: data.scope,
			module: params.name,
			page,
			title,
			version: data.version,
			versions: data.versions,
			latest: data.latest,
			description: data.description,
			html: null as string | null,
			toc: [],
			raw: '',
			sections: [],
			mod,
			guides: data.guides,
			...pager,
			fromGuide: null as string | null,
			noApi: true
		};
	}
	if (!isBase) error(404, 'doc page not found');
	const started = isStd ? stdStarted(full, mod) : registryStarted(full, mod);
	const src =
		page === 'overview'
			? overviewMarkdown(full, mod, data.description)
			: gettingStartedMarkdown(full, {
					importLine: started.importLine,
					snippet: started.snippet,
					installed: isStd
				});
	const { html, toc } = renderMarkdown(src.replace(/^\s*# .*\n/, ''));
	return {
		full,
		scope: data.scope,
		module: params.name,
		page,
		title,
		version: data.version,
		versions: data.versions,
		latest: data.latest,
		description: data.description,
		html: html as string | null,
		toc,
		raw: src,
		sections: splitRawSections(src),
		mod,
		guides: data.guides,
		...pager,
		fromGuide: null as string | null,
		noApi: false
	};
}
