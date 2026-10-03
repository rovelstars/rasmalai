// Builds website/static/data/search-index.json: a unified multi-corpus
// search index over manual chapters, guide chapters, rosetta guides,
// @std package docs, and api.json symbols.
//
// Anchor slugs use the same slugify as src/lib/docs/markdown.ts so deep
// links match the rendered heading ids. Part names mirror MANUAL_PARTS in
// src/lib/docs/nav.ts; update both together.
import { readFileSync, writeFileSync, readdirSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

// Load website/.env for build-time Turso access (Pages also injects
// production env vars into the build environment when configured).
{
	const envFile = join(root, '.env');
	if (existsSync(envFile)) {
		for (const line of readFileSync(envFile, 'utf8').split('\n')) {
			const t = line.trim();
			if (!t || t.startsWith('#')) continue;
			const eq = t.indexOf('=');
			if (eq > 0 && !(t.slice(0, eq).trim() in process.env)) {
				process.env[t.slice(0, eq).trim()] = t.slice(eq + 1).trim();
			}
		}
	}
}
const contentDir = join(root, 'src', 'content');
const dataDir = join(root, 'static', 'data');

const slugify = (t) =>
	t
		.toLowerCase()
		.replace(/[^a-z0-9\s-]/g, '')
		.trim()
		.replace(/\s+/g, '-');

const stripInline = (t) =>
	t
		.replace(/<[^>]*>/g, '')
		.replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
		.replace(/[`*_~]/g, '')
		.trim();

function frontmatter(src) {
	const m = src.match(/^---\n([\s\S]*?)\n---\n/);
	const meta = {};
	if (m) {
		for (const line of m[1].split('\n')) {
			const i = line.indexOf(':');
			if (i > 0) meta[line.slice(0, i).trim()] = line.slice(i + 1).trim().replace(/^"|"$/g, '');
		}
	}
	return { meta, body: m ? src.slice(m[0].length) : src };
}

function sectionText(body) {
	return body
		.replace(/```[\s\S]*?```/g, ' ')
		.replace(/<[^>]*>/g, ' ')
		.replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
		.replace(/[#>*`~|-]/g, ' ')
		.replace(/\s+/g, ' ')
		.trim();
}

const MANUAL_PARTS = [
	'Part I - Syntax and Primitives',
	'Part II - Type System and Object Model',
	'Part III - Memory, Systems, and Concurrency',
	'Part IV - Toolchain and Diagnostics'
];

function manualPart(slug) {
	const n = parseInt(slug.slice(0, 2), 10);
	if (n >= 1 && n <= 5) return MANUAL_PARTS[0];
	if (n >= 6 && n <= 10) return MANUAL_PARTS[1];
	if (n >= 11 && n <= 15) return MANUAL_PARTS[2];
	return MANUAL_PARTS[3];
}

const CHAPTER_ALIASES = {
	'01a-prelude-and-intrinsics': ['prelude', 'builtin', 'implicit', 'print', 'assert', 'typeof', 'option', 'result', 'intrinsics', 'find', 'filter', 'reduce'],
	'01-lexicon-and-structure': ['syntax', 'keyword', 'comment', 'string', 'interpolation', 'bool'],
	'02-numeric-model': ['numeric', 'number', 'int', 'float', 'integer', 'fastfloat', 'bitwise'],
	'03-bindings-and-scope': ['let', 'const', 'binding', 'scope', 'destructuring', 'variable', 'array'],
	'04-control-flow': ['if', 'for', 'while', 'switch', 'defer', 'guard', 'loop', 'control flow', 'fallthrough'],
	'05-functions-and-closures': ['function', 'closure', 'lambda', 'throws', 'try', 'catch', 'option', 'result', 'error handling', 'generic', 'test fn'],
	'06-structs-and-records': ['struct', 'record', 'value type'],
	'07-classes-and-objects': ['class', 'classes', 'object', 'extends', 'inheritance', 'method'],
	'08-traits-and-interfaces': ['trait', 'interface', 'dispatch', 'is', 'typeof'],
	'09-extensions-and-operators': ['extension', 'operator', 'overload', 'op_add', 'op_sub', 'op_index', 'iterator', 'iterable'],
	'10-enums-and-matching': ['enum', 'match', 'pattern', 'variant', 'exhaustive'],
	'11-memory-and-arc': ['arc', 'memory', 'retain', 'release', 'reference counting', 'ownership', 'scope'],
	'12-cycles-and-handles': ['genref', 'cycle', 'weak', 'handle', 'byid', 'decay', 'deinit'],
	'13-hardware-and-ffi': ['unsafe', 'ffi', 'pointer', 'native', 'c-abi', 'file', 'process'],
	'14-concurrency-and-threads': ['thread', 'threads', 'concurrency', 'parallel', 'threadpool', 'async', 'await', 'atomic', 'mutex', 'channel', 'barrier'],
	'15-vectorization-and-simd': ['simd', 'vector', 'vec4f', 'lanes', 'dot'],
	'16-project-and-toolchain': ['project.config', 'project', 'manifest', 'toolchain', 'rnx', 'cli', 'test', 'bench', 'doc', 'semver', 'module', 'import', 'package', 'mcp'],
	'17-diagnostics-directory': ['diagnostic', 'error code', 'warning', 'lint', 'e108', 'e303', 'e304']
};

const STOP = new Set(['and', 'the', 'with', 'for', 'from', 'into', 'your', 'you']);
const tokens = (t) => t.toLowerCase().split(/[\s&/]+/).filter((w) => w && !STOP.has(w));

function indexMarkdownFile(path, base, category, section, aliases) {
	const { meta, body } = frontmatter(readFileSync(path, 'utf8'));
	const title = meta.title ?? base;
	const slug = base.replace(/^\//, '').replace(/\//g, '-');
	const entries = [
		{
			id: slug,
			title,
			url: base === '/' ? '/' : base,
			category,
			section,
			keywords: [...new Set([...tokens(title), ...(aliases ?? [])])],
			content: (meta.description ?? '').slice(0, 240),
			weight: 100
		}
	];
	const lines = body.split('\n');
	let current = null;
	const chunks = [];
	for (const line of lines) {
		const h = line.match(/^(#{2,3})\s+(.*)$/);
		if (h) {
			if (current) current.text = sectionText(current.raw);
			current = { depth: h[1].length, heading: stripInline(h[2]), raw: '' };
			chunks.push(current);
		} else if (current) {
			current.raw += line + '\n';
		}
	}
	if (current) current.text = sectionText(current.raw);
	for (const c of chunks) {
		if (!c.heading) continue;
		entries.push({
			id: `${slug}#${slugify(c.heading)}`,
			title: c.heading,
			url: `${base}#${slugify(c.heading)}`,
			category,
			section: `${section} - ${title}`,
			keywords: [...new Set(tokens(c.heading))],
			content: c.text.slice(0, 240),
			weight: c.depth === 2 ? 80 : 60
		});
	}
	return entries;
}

const entries = [];

// Manual: 17 chapters.
for (const file of readdirSync(join(contentDir, 'manual')).filter((f) => f.endsWith('.md')).sort()) {
	const slug = file.replace(/\.md$/, '');
	entries.push(
		...indexMarkdownFile(
			join(contentDir, 'manual', file),
			`/manual/${slug}`,
			'manual',
			manualPart(slug),
			CHAPTER_ALIASES[slug] ?? []
		)
	);
}

// Guide chapters + rosetta.
const ROSETTA_LABEL = { rust: 'Rust', go: 'Go', cpp: 'C++', typescript: 'TypeScript' };
for (const file of readdirSync(join(contentDir, 'guide')).filter((f) => f.endsWith('.md')).sort()) {
	const slug = file.replace(/\.md$/, '');
	entries.push(...indexMarkdownFile(join(contentDir, 'guide', file), `/guide/${slug}`, 'guide', 'Guide', []));
}
for (const file of readdirSync(join(contentDir, 'guide', 'rosetta')).filter((f) => f.endsWith('.md')).sort()) {
	const slug = file.replace(/\.md$/, '').replace(/^from-/, '');
	entries.push(
		...indexMarkdownFile(
			join(contentDir, 'guide', 'rosetta', file),
			`/guide/rosetta/from-${slug}`,
			'guide',
			`Rosetta - ${ROSETTA_LABEL[slug] ?? slug}`,
			[]
		)
	);
}

// Tracks: 2 entries.
for (const file of readdirSync(join(contentDir, 'tracks')).filter((f) => f.endsWith('.md')).sort()) {
	const slug = file.replace(/\.md$/, '');
	entries.push(...indexMarkdownFile(join(contentDir, 'tracks', file), `/guide/tracks/${slug}`, 'guide', 'Learning track', []));
}

// Package docs + symbols from api.json and stdlib.ts metadata. api.json
// is gitignored and may be absent (cargo unavailable, data not yet
// published); the index then covers prose and registry entries only.
let api = { modules: [] };
try {
	api = JSON.parse(readFileSync(join(dataDir, 'api.json'), 'utf8'));
} catch {
	console.log('search index: no api.json, skipping symbol entries');
}
const stdlibSrc = readFileSync(join(root, 'src', 'lib', 'docs', 'stdlib.ts'), 'utf8');
const metaByModule = {};
for (const m of stdlibSrc.matchAll(/\{\s*name:\s*'([^']+)',\s*tagline:\s*'([^']*)',\s*primaryExport:\s*'([^']*)',\s*whenToUse:\s*'([\s\S]*?)',\s*capabilities:\s*\[([^\]]*)\]/g)) {
	metaByModule[m[1]] = {
		tagline: m[2],
		primaryExport: m[3],
		whenToUse: m[4].replace(/\s+/g, ' '),
		capabilities: [...m[5].matchAll(/'([^']*)'/g)].map((c) => c[1])
	};
}

for (const mod of api.modules) {
	const meta = metaByModule[mod.name] ?? { tagline: '', primaryExport: '', whenToUse: '', capabilities: [] };
	const desc = mod.docs?.description ?? '';
	const symbolNames = [
		...mod.classes.map((c) => c.name),
		...mod.enums.map((e) => e.name),
		...mod.functions.map((f) => f.name)
	];
	const overviewKeywords = new Set([mod.name, 'std', meta.primaryExport.toLowerCase(), ...meta.tagline.toLowerCase().split(/[\s,]+/).filter((w) => w.length > 3)]);
	if (mod.name === 'prelude') {
		for (const k of ['prelude', 'builtin', 'implicit', 'print', 'assert', 'typeof', 'option', 'result', 'find', 'filter', 'reduce', 'map', 'string', 'array']) overviewKeywords.add(k);
	}
	entries.push({
		id: `pkg-${mod.name}-overview`,
		title: `@std/${mod.name}`,
		url: `/docs/@std/${mod.name}/overview`,
		category: 'package',
		section: 'Package',
		keywords: [...overviewKeywords],
		content: `${meta.tagline} ${meta.whenToUse} ${meta.capabilities.join('. ')}`.slice(0, 240),
		weight: 85
	});
	entries.push({
		id: `pkg-${mod.name}-getting-started`,
		title: `@std/${mod.name} getting started`,
		url: `/docs/@std/${mod.name}/getting-started`,
		category: 'package',
		section: 'Package',
		keywords: [...new Set([mod.name, 'std', 'quickstart', 'example', 'import'])],
		content: `Quickstart and import snippet. Symbols: ${symbolNames.join(', ')}`.slice(0, 240),
		weight: 70
	});
	for (const c of mod.classes) {
		const firstLine = (c.docs?.description ?? '').split('\n')[0].trim();
		const parent = `sym-${mod.name}-${c.name}`;
		entries.push({
			id: parent,
			title: c.name,
			url: `/docs/@std/${mod.name}/api#class-${c.name}`,
			category: 'symbol',
			section: `@std/${mod.name}`,
			keywords: [...new Set(['class', mod.name, c.name.toLowerCase()])],
			content: firstLine.slice(0, 240),
			weight: 75
		});
		for (const meth of c.methods ?? []) {
			entries.push({
				id: `${parent}.${meth.name}`,
				title: `${c.name}.${meth.name}`,
				url: `/docs/@std/${mod.name}/api#class-${c.name}`,
				category: 'symbol',
				section: `@std/${mod.name} - ${c.name}`,
				parent,
				keywords: [...new Set([meth.name.toLowerCase(), c.name.toLowerCase(), (meth.sig ?? '').toLowerCase().slice(0, 80)])],
				content: ((meth.docs?.description ?? '').split('\n')[0].trim() + ' ' + (meth.sig ?? '')).slice(0, 240),
				weight: 40
			});
		}
		for (const f of c.fields ?? []) {
			entries.push({
				id: `${parent}.${f.name}#field`,
				title: `${c.name}.${f.name}`,
				url: `/docs/@std/${mod.name}/api#class-${c.name}`,
				category: 'symbol',
				section: `@std/${mod.name} - ${c.name}`,
				parent,
				keywords: [...new Set([f.name.toLowerCase(), c.name.toLowerCase(), 'field'])],
				content: ((f.docs?.description ?? '').split('\n')[0].trim() + ' ' + (f.ty ?? '')).slice(0, 240),
				weight: 40
			});
		}
	}
	for (const fn of mod.functions ?? []) {
		entries.push({
			id: `sym-${mod.name}-${fn.name}`,
			title: `${fn.name}()`,
			url: `/docs/@std/${mod.name}/api#fn-${fn.name}`,
			category: 'symbol',
			section: `@std/${mod.name}`,
			keywords: [...new Set(['function', mod.name, fn.name.toLowerCase()])],
			content: ((fn.docs?.description ?? '').split('\n')[0].trim() + ' ' + (fn.sig ?? '')).slice(0, 240),
			weight: 65
		});
	}
	for (const e of mod.enums ?? []) {
		const eparent = `sym-${mod.name}-${e.name}`;
		entries.push({
			id: eparent,
			title: e.name,
			url: `/docs/@std/${mod.name}/api#enum-${e.name}`,
			category: 'symbol',
			section: `@std/${mod.name}`,
			keywords: [...new Set(['enum', mod.name, e.name.toLowerCase()])],
			content: ((e.docs?.description ?? '').split('\n')[0].trim()).slice(0, 240),
			weight: 65
		});
		for (const v of e.variants ?? []) {
			entries.push({
				id: `${eparent}.${v.name}`,
				title: `${e.name}.${v.name}`,
				url: `/docs/@std/${mod.name}/api#enum-${e.name}`,
				category: 'symbol',
				section: `@std/${mod.name} - ${e.name}`,
				parent: eparent,
				keywords: [...new Set([v.name.toLowerCase(), e.name.toLowerCase(), 'variant'])],
				content: ((v.docs?.description ?? '').split('\n')[0].trim()).slice(0, 240),
				weight: 40
			});
		}
	}
	for (const k of mod.constants ?? []) {
		entries.push({
			id: `sym-${mod.name}-${k.name}`,
			title: k.name,
			url: `/docs/@std/${mod.name}/api`,
			category: 'symbol',
			section: `@std/${mod.name}`,
			keywords: [...new Set(['constant', mod.name, k.name.toLowerCase()])],
			content: ((k.docs?.description ?? '').split('\n')[0].trim()).slice(0, 240),
			weight: 40
		});
	}
}

// Registry packages from the live Turso catalog (same source as the
// packages page). Unavailable at build time without credentials — the
// packages page carries its own filter, so the index degrades gracefully.
try {
	const { createClient } = await import('@libsql/client/web');
	const dbUrl = process.env['TURSO_DATABASE_URL'];
	if (dbUrl) {
		const db = createClient({ url: dbUrl, authToken: process.env['TURSO_AUTH_TOKEN'] });
		const rs = await db.execute(
			"SELECT scope, name, description FROM packages ORDER BY updated_at DESC"
		);
		for (const r of rs.rows) {
			const scope = String(r['scope'] ?? '');
			const name = String(r['name']);
			const full = scope ? `@${scope}/${name}` : name;
			const desc = String(r['description'] ?? '');
			entries.push({
				id: `reg-${full}`,
				title: full,
				url: `/packages/${full}`,
				category: 'package',
				section: 'Registry',
				keywords: [...new Set([full.toLowerCase(), 'package', ...desc.toLowerCase().split(/[\s,]+/).filter((w) => w.length > 3)])],
				content: desc.slice(0, 240),
				weight: 70
			});
		}
	}
} catch {
	/* registry entries skipped */
}

writeFileSync(join(dataDir, 'search-index.json'), JSON.stringify(entries) + '\n');
console.log(`search index: ${entries.length} entries`);
