import { error } from '@sveltejs/kit';
import { renderMarkdown, splitRawSections } from '$lib/docs/markdown';
import {
	STD_MODULES,
	ensureApi,
	stdApiModule,
	stdMeta,
	importSnippet,
	quickstartFor
} from '$lib/docs/stdlib';
import type { DocModule } from '$lib/docs/api';

const PAGES = ['overview', 'getting-started', 'api'] as const;

export function entries() {
	const out: { module: string; page: string }[] = [];
	for (const m of STD_MODULES) {
		for (const page of PAGES) out.push({ module: m.name, page });
	}
	return out;
}

function overviewMarkdown(module: string, mod: DocModule): string {
	const meta = stdMeta(module);
	const lines = [
		`# @std/${module} overview`,
		'',
		mod.docs.description,
		'',
		'## When to use it',
		'',
		meta.whenToUse,
		'',
		'## Capabilities',
		'',
		...meta.capabilities.map((c) => `- ${c}`),
		'',
		'## Symbols',
		'',
		...mod.classes.map((c) => `- class **${c.name}** — ${(c.docs.description.split('\n')[0] ?? '').trim()}`),
		...mod.enums.map((e) => `- enum **${e.name}**`),
		...mod.functions.map((f) => `- fn **${f.name}()**`)
	];
	return lines.join('\n');
}

function gettingStartedMarkdown(module: string): string {
	const quickstart = quickstartFor(module);
	return [
		`# @std/${module} getting started`,
		'',
		'No install step: the module ships inside the compiler. Import what you need:',
		'',
		'```rnx',
		importSnippet(module),
		'```',
		'',
		'## Quickstart',
		'',
		'Copy this into a file or open it straight in the playground:',
		'',
		'```rnx',
		quickstart,
		'```'
	].join('\n');
}

export async function load({ params, fetch }) {
	if (!PAGES.includes(params.page as (typeof PAGES)[number])) error(404, 'doc page not found');
	if (!STD_MODULES.some((m) => m.name === params.module)) error(404, 'package not found');
	await ensureApi(fetch);
	const mod = stdApiModule(params.module);
	if (params.page === 'api') {
		const toc = mod
			? [
					...mod.classes.map((c) => ({ id: `class-${c.name}`, text: c.name, level: 2 })),
					...mod.enums.map((e) => ({ id: `enum-${e.name}`, text: e.name, level: 2 })),
					...mod.functions.map((f) => ({ id: `fn-${f.name}`, text: `${f.name}()`, level: 2 }))
				]
			: [];
		return {
			module: params.module,
			page: params.page,
			html: null as string | null,
			toc,
			mod,
			noApi: mod === null
		};
	}
	if (!mod) {
		return {
			module: params.module,
			page: params.page,
			html: null as string | null,
			toc: [],
			raw: '',
			sections: [],
			mod,
			noApi: true
		};
	}
	const src = params.page === 'overview' ? overviewMarkdown(params.module, mod) : gettingStartedMarkdown(params.module);
	const { html, toc } = renderMarkdown(src.replace(/^\s*# .*\n/, ''));
	return {
		module: params.module,
		page: params.page,
		html: html as string | null,
		toc,
		raw: src,
		sections: splitRawSections(src),
		mod,
		noApi: false
	};
}
