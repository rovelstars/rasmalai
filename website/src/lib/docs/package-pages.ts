export interface PackagePageInput {
	slug: string;
	title: string;
	section?: string | null;
	order?: string | null;
}

export interface PackagePageLink {
	slug: string;
	title: string;
}

export interface PackagePageSection {
	section: string;
	pages: PackagePageLink[];
}

export interface PackagePageNav {
	loose: PackagePageLink[];
	sections: PackagePageSection[];
	order: PackagePageLink[];
}

const BASE_PAGES: PackagePageLink[] = [
	{ slug: 'overview', title: 'Overview' },
	{ slug: 'getting-started', title: 'Getting started' }
];

const API_PAGE: PackagePageLink = { slug: 'api', title: 'API Reference' };

function rank(page: PackagePageInput): [number, number, string] {
	const order = page.order ?? '';
	if (order !== '' && !Number.isNaN(Number(order))) return [0, Number(order), page.slug];
	return [1, 0, page.slug];
}

function compare(a: PackagePageInput, b: PackagePageInput): number {
	const ar = rank(a);
	const br = rank(b);
	if (ar[0] !== br[0]) return ar[0] - br[0];
	if (ar[1] !== br[1]) return ar[1] - br[1];
	return ar[2] < br[2] ? -1 : ar[2] > br[2] ? 1 : 0;
}

function link(page: PackagePageInput): PackagePageLink {
	return { slug: page.slug, title: page.title };
}

export function buildPackagePageNav(guides: PackagePageInput[]): PackagePageNav {
	const baseSlugs = new Set([...BASE_PAGES.map((p) => p.slug), API_PAGE.slug]);
	const loose: PackagePageInput[] = [];
	const bySection = new Map<string, PackagePageInput[]>();
	for (const guide of guides) {
		if (baseSlugs.has(guide.slug)) continue;
		const section = guide.section?.trim();
		if (!section) {
			loose.push(guide);
			continue;
		}
		const pages = bySection.get(section) ?? [];
		pages.push(guide);
		bySection.set(section, pages);
	}

	const sections: { section: string; pages: PackagePageInput[] }[] = [];
	for (const [section, pages] of bySection) {
		pages.sort(compare);
		if (pages.length === 1) loose.push(pages[0]);
		else sections.push({ section, pages });
	}
	loose.sort(compare);
	sections.sort((a, b) => compare(a.pages[0], b.pages[0]));

	const looseLinks = loose.map(link);
	const sectionLinks = sections.map((section) => ({
		section: section.section,
		pages: section.pages.map(link)
	}));
	return {
		loose: looseLinks,
		sections: sectionLinks,
		order: [
			...BASE_PAGES,
			...looseLinks,
			...sectionLinks.flatMap((section) => section.pages),
			API_PAGE
		]
	};
}
