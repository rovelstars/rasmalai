import { marked } from 'marked';
import { sanitizeGuideHtml } from '$lib/server/sanitize';

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
