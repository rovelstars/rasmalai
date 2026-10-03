export interface SearchEntry {
	id: string;
	title: string;
	url: string;
	category: 'manual' | 'guide' | 'package' | 'symbol';
	section?: string;
	parent?: string;
	keywords: string[];
	content: string;
	weight: number;
}

export interface ScoredEntry extends SearchEntry {
	score: number;
}

function fieldScore(text: string, token: string): number {
	const t = text.toLowerCase();
	if (t === token) return 1000;
	if (t.startsWith(token)) return 500;
	if (t.includes(token)) return 300;
	return 0;
}

function keywordScore(keywords: string[], token: string): number {
	let best = 0;
	for (const k of keywords) {
		const kk = k.toLowerCase();
		if (kk === token) return 400;
		if (kk.startsWith(token)) best = Math.max(best, 200);
		else if (kk.includes(token)) best = Math.max(best, 150);
	}
	return best;
}

const escapeRegExp = (t: string): string => t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

export function scoreEntry(e: SearchEntry, tokens: string[]): number {
	let field = 0;
	for (const token of tokens) {
		const wordHit = new RegExp(`\\b${escapeRegExp(token)}`, 'i').test(e.content);
		const best = Math.max(
			fieldScore(e.title, token),
			keywordScore(e.keywords, token),
			wordHit ? 50 : 0
		);
		if (best === 0) return 0;
		field += best;
	}
	return e.weight + field;
}

function childName(e: SearchEntry): string {
	const i = e.title.lastIndexOf('.');
	return (i >= 0 ? e.title.slice(i + 1) : e.title).toLowerCase().replace(/\(\)$/, '');
}

export function searchEntries(entries: SearchEntry[], query: string, limit = 12): ScoredEntry[] {
	const tokens = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
	if (tokens.length === 0) return entries.slice(0, limit).map((e) => ({ ...e, score: 0 }));
	const ranked = entries
		.map((e) => ({ ...e, score: scoreEntry(e, tokens) }))
		.filter((e) => e.score > 0)
		.sort((a, b) => b.score - a.score || a.title.localeCompare(b.title));
	const out: ScoredEntry[] = [];
	const perParent = new Map<string, number>();
	for (const e of ranked) {
		if (out.length >= limit) break;
		if (e.parent) {
			const name = childName(e);
			const explicit = tokens.some((t) => name.startsWith(t) || (t.length > 2 && name.includes(t)));
			if (!explicit) {
				const used = perParent.get(e.parent) ?? 0;
				if (used >= 3) continue;
				perParent.set(e.parent, used + 1);
			}
		}
		out.push(e);
	}
	return out;
}
