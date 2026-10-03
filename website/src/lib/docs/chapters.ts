import { parseFrontmatter } from './markdown';

export interface DerivedChapter {
	slug: string;
	title: string;
	description: string;
	icon: string | null;
	part: string | null;
	path: string;
}

type Rank = [number, number, string];

function rank(slug: string, order: string | undefined): Rank {
	if (order !== undefined && order !== '' && !Number.isNaN(Number(order))) return [0, Number(order), slug];
	return [1, 0, slug];
}

function cmp(a: Rank, b: Rank): number {
	if (a[0] !== b[0]) return a[0] - b[0];
	if (a[1] !== b[1]) return a[1] - b[1];
	return a[2] < b[2] ? -1 : a[2] > b[2] ? 1 : 0;
}

export function buildChapters(files: Record<string, string>, basePath: string): DerivedChapter[] {
	const scored: { chapter: DerivedChapter; rank: Rank }[] = [];
	for (const [file, raw] of Object.entries(files)) {
		const at = file.lastIndexOf(basePath);
		const tail = at >= 0 ? file.slice(at + basePath.length) : (file.split('/').pop() ?? file);
		const clean = tail.replace(/^\/+/, '');
		if (clean.includes('/') || !clean.endsWith('.md')) continue;
		const slug = clean.slice(0, -3);
		const { meta } = parseFrontmatter(raw);
		if (!meta.title) throw new Error(`docs nav: ${file} is missing frontmatter title`);
		scored.push({
			rank: rank(slug, meta.order),
			chapter: {
				slug,
				title: meta.title,
				description: meta.description ?? '',
				icon: meta.icon ?? null,
				part: meta.section ?? null,
				path: `${basePath}/${slug}`
			}
		});
	}
	scored.sort((a, b) => cmp(a.rank, b.rank));
	return scored.map((s) => s.chapter);
}
