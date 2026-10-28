import { error, redirect } from '@sveltejs/kit';
import { renderMarkdown, splitRawSections } from '$lib/docs/markdown';
import {
	genericSnippet,
	gettingStartedMarkdown,
	moduleFor,
	overviewMarkdown
} from '$lib/docs/module-doc';
import { getPackage, getVersionDoc } from '$lib/server/db';
import { parseStoredGuides } from '$lib/server/guides';
import type { DocModule } from '$lib/docs/api';

const PAGES = ['overview', 'getting-started', 'api'] as const;

const SCOPE_RE = /^@[a-z0-9][a-z0-9-]*$/;
const NAME_RE = /^[a-z0-9][a-z0-9-]*$/;

const RESERVED = new Set(['tracks', 'coming-from']);

interface GuideOut {
	slug: string;
	title: string;
	html: string;
	source: string;
}

function normalizeModule(m: DocModule): DocModule {
	return {
		name: m.name,
		docs: m.docs ?? { description: '', tags: [] },
		functions: Array.isArray(m.functions) ? m.functions : [],
		classes: Array.isArray(m.classes) ? m.classes : [],
		enums: Array.isArray(m.enums) ? m.enums : [],
		constants: Array.isArray((m as DocModule).constants) ? (m as DocModule).constants : []
	};
}

function modsFromDocJson(docJson: string): DocModule[] {
	try {
		const v = JSON.parse(docJson) as { modules?: unknown };
		if (!v || !Array.isArray(v.modules)) return [];
		return (v.modules as DocModule[])
			.filter((m) => m && typeof m.name === 'string')
			.map(normalizeModule);
	} catch {
		return [];
	}
}

function guidesFromStored(raw: string): GuideOut[] {
	return parseStoredGuides(raw)
		.filter((g) => typeof g.html === 'string')
		.map((g) => ({ slug: g.slug, title: g.title, html: g.html, source: g.source }));
}

function registryStarted(full: string, mod: DocModule | null): { importLine: string; snippet: string } {
	const line = genericSnippet(full, mod);
	if (line.startsWith('//')) return { importLine: line, snippet: line };
	return {
		importLine: line,
		snippet: `${line}\n// exact call shapes live in the API reference below`
	};
}

export async function load({ params, platform, setHeaders }) {
	const scope = params.scope ?? '';
	const name = params.name ?? '';
	if (SCOPE_RE.test(scope) && NAME_RE.test(name)) {
		redirect(308, `/docs/${scope}/${name}/overview`);
	}
	if (!NAME_RE.test(scope) || RESERVED.has(scope)) error(404, 'package not found');
	if (!(PAGES as readonly string[]).includes(name)) error(404, 'doc page not found');
	const full = scope;
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const pkg = await getPackage(env, full);
	if (!pkg) error(404, 'package not found');
	const meta = pkg.versions[0];
	const doc = await getVersionDoc(env, pkg.name, meta.version);
	if (!doc) error(404, 'version not found');
	setHeaders({ 'Cache-Control': 'public, max-age=300, s-maxage=300' });
	const mods = modsFromDocJson(doc.docJson);
	const guides = guidesFromStored(doc.guides);
	const base = {
		full,
		scope: '',
		module: scope,
		page: name,
		version: meta.version,
		description: pkg.description,
		mod: moduleFor(mods, scope),
		guides
	};
	if (name === 'api') {
		const mod = base.mod;
		const toc = mod
			? [
					...mod.classes.map((c) => ({ id: `class-${c.name}`, text: c.name, level: 2 })),
					...mod.enums.map((e) => ({ id: `enum-${e.name}`, text: e.name, level: 2 })),
					...mod.functions.map((f) => ({ id: `fn-${f.name}`, text: `${f.name}()`, level: 2 }))
				]
			: [];
		return { ...base, html: null as string | null, toc, noApi: mod === null };
	}
	if (!base.mod) {
		return {
			...base,
			html: null as string | null,
			toc: [],
			raw: '',
			sections: [],
			fromGuide: null as string | null,
			noApi: true
		};
	}
	const started = registryStarted(full, base.mod);
	const src =
		name === 'overview'
			? overviewMarkdown(full, base.mod, pkg.description)
			: gettingStartedMarkdown(full, {
					importLine: started.importLine,
					snippet: started.snippet,
					installed: false
				});
	const stored = guides.find((g) => g.slug === name);
	if (stored && stored.html) {
		const rendered = renderMarkdown(stored.source.replace(/^\s*# .*\n/, ''));
		return {
			...base,
			html: stored.html,
			toc: rendered.toc,
			raw: stored.source,
			sections: splitRawSections(stored.source),
			fromGuide: stored.slug,
			noApi: false
		};
	}
	const { html, toc } = renderMarkdown(src.replace(/^\s*# .*\n/, ''));
	return {
		...base,
		html: html as string | null,
		toc,
		raw: src,
		sections: splitRawSections(src),
		fromGuide: null as string | null,
		noApi: false
	};
}
