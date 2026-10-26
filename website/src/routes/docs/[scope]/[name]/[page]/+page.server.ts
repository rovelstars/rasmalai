import { error } from '@sveltejs/kit';
import { getPackage, getVersionDoc } from '$lib/server/db';
import { parseStoredGuides } from '$lib/server/guides';
import type { DocModule } from '$lib/docs/api';

export const prerender = false;

const PAGES = ['overview', 'getting-started', 'api'] as const;

const SCOPE_RE = /^@[a-z0-9][a-z0-9-]*$/;
const NAME_RE = /^[a-z0-9][a-z0-9-]*$/;

// Top-level @std modules with module-doc pages. Mirrors STDLIB in
// $lib/docs/nav and the tops of MODULES in compiler/stdlib/src/lib.rs;
// docs-routes.test.ts pins the three lists together.
const STD_TOPS = [
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

export async function load({ params, platform, url, fetch, setHeaders }) {
	if (!PAGES.includes(params.page as (typeof PAGES)[number])) error(404, 'doc page not found');
	const scope = params.scope ?? '';
	const name = params.name ?? '';
	if (!SCOPE_RE.test(scope) || !NAME_RE.test(name)) error(404, 'package not found');
	const full = `${scope}/${name}`;
	if (scope === '@std' && !STD_TOPS.includes(name)) error(404, 'package not found');
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	let isDev = false;
	try {
		isDev = (await import('$app/environment')).dev === true;
	} catch {
		isDev = false;
	}

	if (isDev && full.startsWith('@std/')) {
		const { localStdPackage } = await import('$lib/server/std-local');
		const local = localStdPackage(full);
		if (!local) error(404, 'package not found');
		setHeaders({ 'Cache-Control': 'no-store' });
		return {
			full,
			scope,
			name,
			page: params.page,
			version: local.doc.version,
			description: local.pkg.description,
			mods: modsFromDocJson(local.doc.docJson),
			guides: guidesFromStored(local.doc.guides)
		};
	}

	try {
		const pkg = await getPackage(env, full);
		if (pkg) {
			const meta = pkg.versions[0];
			const doc = await getVersionDoc(env, pkg.name, meta.version);
			if (doc) {
				setHeaders({ 'Cache-Control': 'public, max-age=300, s-maxage=300' });
				return {
					full,
					scope,
					name,
					page: params.page,
					version: meta.version,
					description: pkg.description,
					mods: modsFromDocJson(doc.docJson),
					guides: guidesFromStored(doc.guides)
				};
			}
		}
	} catch {
		// fall through to the std static fallback below
	}

	if (full.startsWith('@std/')) {
		try {
			const res = await fetch(new URL('/data/api.json', url.origin));
			if (res.ok) {
				const snap = (await res.json()) as { version?: string; modules?: DocModule[] };
				const mods = Array.isArray(snap.modules)
					? snap.modules.filter((m) => m && typeof m.name === 'string').map(normalizeModule)
					: [];
				if (mods.length > 0) {
					const mod = mods.find((m) => m.name === name) ?? null;
					const firstLine = (mod?.docs.description ?? '').split('\n')[0];
					setHeaders({ 'Cache-Control': 'public, max-age=300, s-maxage=300' });
					return {
						full,
						scope,
						name,
						page: params.page,
						version: typeof snap.version === 'string' ? snap.version : '0.1.0',
						description: firstLine || `${full} standard library module`,
						mods,
						guides: [] as GuideOut[]
					};
				}
			}
		} catch {
			// fall through to 404
		}
	}
	error(404, 'package not found');
}
