import { error } from '@sveltejs/kit';
import { ROSETTA } from '$lib/docs/nav';
import { parseFrontmatter, splitSections } from '$lib/docs/markdown';
import fromRust from '../../../../content/guide/rosetta/from-rust.md?raw';
import fromTypescript from '../../../../content/guide/rosetta/from-typescript.md?raw';
import fromGo from '../../../../content/guide/rosetta/from-go.md?raw';
import fromCpp from '../../../../content/guide/rosetta/from-cpp.md?raw';

const SOURCES: Record<string, string> = {
	'from-rust': fromRust,
	'from-typescript': fromTypescript,
	'from-go': fromGo,
	'from-cpp': fromCpp
};

export function entries() {
	return Object.keys(SOURCES).map((slug) => ({ slug }));
}

export async function load({ params }) {
	const src = SOURCES[params.slug];
	if (!src) error(404, 'rosetta guide not found');
	const { meta } = parseFrontmatter(src);
	const { sections, toc } = splitSections(src);
	const title = meta.title || params.slug;
	return {
		slug: params.slug,
		title,
		description: meta.description,
		sourceLang: meta.sourceLang ?? '',
		sections,
		toc
	};
}
