import type { DerivedChapter } from './chapters';

export interface PackageGuideInput {
	slug: string;
	title: string;
	description: string;
	section?: string | null;
	order?: string | null;
}

export interface PackageGuideDoc {
	slug: string;
	title: string;
	description: string;
	section: string | null;
	order: string | null;
	markdown: string;
}

export interface GuidesJson {
	version: number;
	guides: PackageGuideDoc[];
}

export interface PackageNavGroup {
	section: string;
	chapters: DerivedChapter[];
}

type Rank = [number, number, string];

function rank(slug: string, order: string | null | undefined): Rank {
	if (order !== undefined && order !== null && order !== '' && !Number.isNaN(Number(order)))
		return [0, Number(order), slug];
	return [1, 0, slug];
}

function cmp(a: Rank, b: Rank): number {
	if (a[0] !== b[0]) return a[0] - b[0];
	if (a[1] !== b[1]) return a[1] - b[1];
	return a[2] < b[2] ? -1 : a[2] > b[2] ? 1 : 0;
}

function sectionOf(guide: PackageGuideInput): string | null {
	const s = guide.section?.trim();
	return s ? s : null;
}

export function buildPackageNav(
	guides: (PackageGuideInput & Record<string, unknown>)[],
	apiModules: string[]
): PackageNavGroup[] {
	const scored: { chapter: DerivedChapter; rank: Rank }[] = [];
	for (const g of guides) {
		const part = sectionOf(g);
		scored.push({
			rank: rank(g.slug, (g.order ?? null) as string | null),
			chapter: {
				slug: g.slug,
				title: g.title,
				description: g.description ?? '',
				icon: null,
				part,
				path: g.slug
			}
		});
	}
	scored.sort((a, b) => cmp(a.rank, b.rank));

	const order: (string | null)[] = [];
	const bySection = new Map<string | null, DerivedChapter[]>();
	for (const s of scored) {
		const key = s.chapter.part;
		if (!bySection.has(key)) {
			bySection.set(key, []);
			order.push(key);
		}
		bySection.get(key)?.push(s.chapter);
	}

	const named = order.filter((k): k is string => k !== null);
	if (bySection.has(null)) named.push('');
	const groups: PackageNavGroup[] = [];
	for (const key of named) {
		const chapters = key === '' ? (bySection.get(null) ?? []) : (bySection.get(key) ?? []);
		groups.push({
			section: key === '' ? '' : key,
			chapters
		});
	}

	const modules = [...new Set(apiModules)].sort();
	groups.push({
		section: 'API Reference',
		chapters: modules.map((m) => ({
			slug: m,
			title: m,
			description: '',
			icon: null,
			part: 'API Reference',
			path: `api/${m}`
		}))
	});
	return groups;
}

export function guidesJsonToInputs(doc: GuidesJson): PackageGuideInput[] {
	return doc.guides.map((g) => ({
		slug: g.slug,
		title: g.title,
		description: g.description,
		section: g.section,
		order: g.order
	}));
}
