import { marked } from 'marked';
import { sanitizeGuideHtml } from '$lib/server/sanitize';
import { MAX_DOC_JSON_BYTES } from '$lib/server/db';

export interface GuideEntry {
	slug: string;
	title: string;
	html: string;
	source: string;
	description: string;
	section: string | null;
	order: string | null;
}

const SLUG_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;

function optText(v: unknown): string | null {
	if (typeof v !== 'string') return null;
	const t = v.trim();
	return t === '' ? null : v;
}

export function normalizeGuideEntry(g: unknown): { ok: true; entry: GuideEntry } | { ok: false; error: string } {
	if (!g || typeof g !== 'object') return { ok: false, error: 'invalid guide entry' };
	const e = g as Record<string, unknown>;
	if (typeof e['slug'] !== 'string' || typeof e['title'] !== 'string') {
		return { ok: false, error: 'guide needs slug and title' };
	}
	if (!SLUG_RE.test(e['slug'])) return { ok: false, error: 'invalid guide slug' };
	const source = typeof e['source'] === 'string' ? e['source'] : '';
	const html = sanitizeGuideHtml(String(marked.parse(source)));
	const description = typeof e['description'] === 'string' ? e['description'] : '';
	return {
		ok: true,
		entry: {
			slug: e['slug'],
			title: e['title'],
			html,
			source,
			description,
			section: optText(e['section']),
			order: optText(e['order'])
		}
	};
}

export function parseStoredGuides(raw: string): GuideEntry[] {
	try {
		const v = JSON.parse(raw) as unknown;
		if (!Array.isArray(v)) return [];
		return v.filter(
			(g): g is GuideEntry => !!g && typeof (g as GuideEntry).slug === 'string' && typeof (g as GuideEntry).title === 'string'
		);
	} catch {
		return [];
	}
}

export interface ApiModuleRef {
	name: string;
}

export function parseApiModules(docJson: string): ApiModuleRef[] {
	try {
		const v = JSON.parse(docJson) as { modules?: Array<{ name?: unknown }> };
		if (!v || !Array.isArray(v.modules)) return [];
		return v.modules.filter((m) => m && typeof m.name === 'string').map((m) => ({ name: m.name as string }));
	} catch {
		return [];
	}
}

const MAX_DOC_MODULES = 4096;
const MAX_DOC_NAME_LEN = 256;
const MAX_DOC_PATH_LEN = 1024;
const MAX_DOC_TEXT_LEN = 64 * 1024;
const MAX_DOC_SIG_LEN = 4096;
const MAX_DOC_TAG_TEXT_LEN = 8192;
const MAX_DOC_TAGS = 4096;
const MAX_DOC_ENTRIES = 16384;

export type DocJsonResult =
	| { ok: true; docJson: string }
	| { ok: false; status: 400 | 413; error: string };

function docFail(status: 400 | 413, error: string): DocJsonResult {
	return { ok: false, status, error };
}

// Plain-text fields inside doc JSON are rendered through the markdown
// renderer (which HTML-escapes), never injected as raw HTML. Strip control
// characters that would otherwise survive into stored snapshots.
function cleanDocText(v: unknown, max: number): string | null {
	if (typeof v !== 'string') return null;
	if (v.length > max) return null;
	let out = '';
	for (let i = 0; i < v.length; i++) {
		const c = v.charCodeAt(i);
		if (c === 10 || c === 9 || c >= 32) out += v[i];
	}
	return out;
}

function readJsDoc(v: unknown): { description: string; tags: Array<{ kind: string; name: string; text: string }> } | null {
	if (!v || typeof v !== 'object' || Array.isArray(v)) return null;
	const t = v as Record<string, unknown>;
	const description = cleanDocText(t['description'], MAX_DOC_TEXT_LEN);
	if (description === null) return null;
	const rawTags = t['tags'];
	if (!Array.isArray(rawTags) || rawTags.length > MAX_DOC_TAGS) return null;
	const tags: Array<{ kind: string; name: string; text: string }> = [];
	for (const tag of rawTags) {
		if (!tag || typeof tag !== 'object' || Array.isArray(tag)) return null;
		const item = tag as Record<string, unknown>;
		const kind = cleanDocText(item['kind'], MAX_DOC_SIG_LEN);
		const name = cleanDocText(item['name'], MAX_DOC_SIG_LEN);
		const text = cleanDocText(item['text'], MAX_DOC_TAG_TEXT_LEN);
		if (kind === null || name === null || text === null) return null;
		tags.push({ kind, name, text });
	}
	return { description, tags };
}

interface DocFnJson {
	name: string;
	sig: string;
	docs: { description: string; tags: Array<{ kind: string; name: string; text: string }> };
}

function readDocFn(v: unknown): DocFnJson | null {
	if (!v || typeof v !== 'object' || Array.isArray(v)) return null;
	const t = v as Record<string, unknown>;
	const name = cleanDocText(t['name'], MAX_DOC_NAME_LEN);
	const sig = cleanDocText(t['sig'], MAX_DOC_SIG_LEN);
	const docs = readJsDoc(t['docs']);
	if (name === null || name === '' || sig === null || docs === null) return null;
	return { name, sig, docs };
}

function readDocList(v: unknown): unknown[] | null {
	if (!Array.isArray(v) || v.length > MAX_DOC_ENTRIES) return null;
	return v;
}

// Client doc snapshots are accepted only in the compiler docgen shape
// (`{"modules":[...]}` as emitted by `rnx publish`, metaVersion 3) and are
// rebuilt key by key: unknown fields - including any pre-rendered `html` -
// are dropped, never stored or served.
export function normalizeDocJson(raw: unknown): DocJsonResult {
	let value: unknown = raw;
	if (typeof value === 'string') {
		if (value.length > MAX_DOC_JSON_BYTES) {
			return docFail(413, `docs exceed ${MAX_DOC_JSON_BYTES} bytes`);
		}
		try {
			value = JSON.parse(value) as unknown;
		} catch {
			return docFail(400, 'invalid docs: not JSON');
		}
	}
	if (!value || typeof value !== 'object' || Array.isArray(value)) {
		return docFail(400, 'invalid docs: top level must be an object with a modules array');
	}
	const modulesRaw = (value as Record<string, unknown>)['modules'];
	if (!Array.isArray(modulesRaw)) {
		return docFail(400, 'invalid docs: missing modules array');
	}
	if (modulesRaw.length > MAX_DOC_MODULES) {
		return docFail(400, `invalid docs: too many modules (${modulesRaw.length})`);
	}
	const modules: Array<Record<string, unknown>> = [];
	for (const m of modulesRaw) {
		if (!m || typeof m !== 'object' || Array.isArray(m)) {
			return docFail(400, 'invalid docs: module must be an object');
		}
		const t = m as Record<string, unknown>;
		const name = cleanDocText(t['name'], MAX_DOC_NAME_LEN);
		const path = cleanDocText(t['path'], MAX_DOC_PATH_LEN);
		if (name === null || name === '' || path === null) {
			return docFail(400, 'invalid docs: module needs a string name and path');
		}
		const docs = readJsDoc(t['docs']);
		if (docs === null) return docFail(400, `invalid docs: module \`${name}\` needs docs { description, tags }`);
		const functions: DocFnJson[] = [];
		const fnsRaw = readDocList(t['functions']);
		if (fnsRaw === null) return docFail(400, `invalid docs: module \`${name}\` needs a functions array`);
		for (const f of fnsRaw) {
			const fn = readDocFn(f);
			if (!fn) return docFail(400, `invalid docs: bad function entry in module \`${name}\``);
			functions.push(fn);
		}
		const classes: Array<Record<string, unknown>> = [];
		const classesRaw = readDocList(t['classes']);
		if (classesRaw === null) return docFail(400, `invalid docs: module \`${name}\` needs a classes array`);
		for (const c of classesRaw) {
			if (!c || typeof c !== 'object' || Array.isArray(c)) {
				return docFail(400, `invalid docs: bad class entry in module \`${name}\``);
			}
			const ct = c as Record<string, unknown>;
			const cname = cleanDocText(ct['name'], MAX_DOC_NAME_LEN);
			const cdocs = readJsDoc(ct['docs']);
			if (cname === null || cname === '' || cdocs === null) {
				return docFail(400, `invalid docs: bad class entry in module \`${name}\``);
			}
			const initRaw = ct['init'];
			let init: string | null = null;
			if (initRaw !== null && initRaw !== undefined) {
				const s = cleanDocText(initRaw, MAX_DOC_SIG_LEN);
				if (s === null) return docFail(400, `invalid docs: bad class init in module \`${name}\``);
				init = s;
			}
			const fields: Array<Record<string, unknown>> = [];
			const fieldsRaw = readDocList(ct['fields']);
			if (fieldsRaw === null) return docFail(400, `invalid docs: bad class fields in module \`${name}\``);
			for (const f of fieldsRaw) {
				if (!f || typeof f !== 'object' || Array.isArray(f)) {
					return docFail(400, `invalid docs: bad class field in module \`${name}\``);
				}
				const ft = f as Record<string, unknown>;
				const fname = cleanDocText(ft['name'], MAX_DOC_NAME_LEN);
				const fty = cleanDocText(ft['ty'], MAX_DOC_SIG_LEN);
				const fdocs = readJsDoc(ft['docs']);
				if (fname === null || fname === '' || fty === null || fdocs === null) {
					return docFail(400, `invalid docs: bad class field in module \`${name}\``);
				}
				fields.push({ name: fname, ty: fty, docs: fdocs });
			}
			const methods: DocFnJson[] = [];
			const methodsRaw = readDocList(ct['methods']);
			if (methodsRaw === null) return docFail(400, `invalid docs: bad class methods in module \`${name}\``);
			for (const f of methodsRaw) {
				const fn = readDocFn(f);
				if (!fn) return docFail(400, `invalid docs: bad class method in module \`${name}\``);
				methods.push(fn);
			}
			classes.push({ name: cname, docs: cdocs, init, fields, methods });
		}
		const enums: Array<Record<string, unknown>> = [];
		const enumsRaw = readDocList(t['enums']);
		if (enumsRaw === null) return docFail(400, `invalid docs: module \`${name}\` needs an enums array`);
		for (const e of enumsRaw) {
			if (!e || typeof e !== 'object' || Array.isArray(e)) {
				return docFail(400, `invalid docs: bad enum entry in module \`${name}\``);
			}
			const et = e as Record<string, unknown>;
			const ename = cleanDocText(et['name'], MAX_DOC_NAME_LEN);
			const edocs = readJsDoc(et['docs']);
			if (ename === null || ename === '' || edocs === null) {
				return docFail(400, `invalid docs: bad enum entry in module \`${name}\``);
			}
			const variants: Array<Record<string, unknown>> = [];
			const variantsRaw = readDocList(et['variants']);
			if (variantsRaw === null) return docFail(400, `invalid docs: bad enum variants in module \`${name}\``);
			for (const x of variantsRaw) {
				if (!x || typeof x !== 'object' || Array.isArray(x)) {
					return docFail(400, `invalid docs: bad enum variant in module \`${name}\``);
				}
				const xt = x as Record<string, unknown>;
				const xname = cleanDocText(xt['name'], MAX_DOC_NAME_LEN);
				const xdocs = readJsDoc(xt['docs']);
				if (xname === null || xname === '' || xdocs === null) {
					return docFail(400, `invalid docs: bad enum variant in module \`${name}\``);
				}
				const payloadRaw = xt['payload'];
				if (!Array.isArray(payloadRaw) || payloadRaw.length > 256) {
					return docFail(400, `invalid docs: bad enum payload in module \`${name}\``);
				}
				const payload: string[] = [];
				for (const p of payloadRaw) {
					const s = cleanDocText(p, MAX_DOC_SIG_LEN);
					if (s === null) return docFail(400, `invalid docs: bad enum payload in module \`${name}\``);
					payload.push(s);
				}
				variants.push({ name: xname, payload, docs: xdocs });
			}
			enums.push({ name: ename, docs: edocs, variants });
		}
		const constants: Array<Record<string, unknown>> = [];
		const constsRaw = readDocList(t['constants']);
		if (constsRaw === null) return docFail(400, `invalid docs: module \`${name}\` needs a constants array`);
		for (const c of constsRaw) {
			if (!c || typeof c !== 'object' || Array.isArray(c)) {
				return docFail(400, `invalid docs: bad constant entry in module \`${name}\``);
			}
			const ct = c as Record<string, unknown>;
			const cname = cleanDocText(ct['name'], MAX_DOC_NAME_LEN);
			const cty = cleanDocText(ct['ty'], MAX_DOC_SIG_LEN);
			const cdocs = readJsDoc(ct['docs']);
			if (cname === null || cname === '' || cty === null || cdocs === null) {
				return docFail(400, `invalid docs: bad constant entry in module \`${name}\``);
			}
			constants.push({ name: cname, ty: cty, docs: cdocs });
		}
		modules.push({ name, path, docs, functions, classes, enums, constants });
	}
	const docJson = JSON.stringify({ modules });
	if (docJson.length > MAX_DOC_JSON_BYTES) {
		return docFail(413, `docs exceed ${MAX_DOC_JSON_BYTES} bytes`);
	}
	return { ok: true, docJson };
}
