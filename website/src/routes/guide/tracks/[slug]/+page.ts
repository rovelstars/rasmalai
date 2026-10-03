import { error } from '@sveltejs/kit';
import { parseFrontmatter, splitSections } from '$lib/docs/markdown';
import gentleRamp from '../../../../content/tracks/gentle-ramp.md?raw';
import fastTrack from '../../../../content/tracks/fast-track.md?raw';

const SOURCES: Record<string, string> = {
	'gentle-ramp': gentleRamp,
	'fast-track': fastTrack
};

export function entries() {
	return Object.keys(SOURCES).map((slug) => ({ slug }));
}

export async function load({ params }) {
	const src = SOURCES[params.slug];
	if (!src) error(404, 'track not found');
	const { meta } = parseFrontmatter(src);
	const { sections, toc } = splitSections(src);
	const title = meta.title || params.slug;
	return { slug: params.slug, title, description: meta.description, sections, toc };
}
