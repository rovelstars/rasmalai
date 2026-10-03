import { error, redirect } from '@sveltejs/kit';
import { MANUAL_SECTIONS, LEGACY_MANUAL_REDIRECTS } from '$lib/docs/nav';
import { parseFrontmatter, renderMarkdown, splitRawSections } from '$lib/docs/markdown';
const SOURCE_FILES = import.meta.glob('../../../content/manual/*.md', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

const SOURCES: Record<string, string> = Object.fromEntries(
	Object.entries(SOURCE_FILES).map(([file, raw]) => [file.split('/').pop()!.slice(0, -3), raw])
);
export function entries() {
	return MANUAL_SECTIONS.map((m) => ({ section: m.slug }));
}

export async function load({ params }) {
	const legacy = LEGACY_MANUAL_REDIRECTS[params.section];
	if (legacy) redirect(308, legacy);
	const meta = MANUAL_SECTIONS.find((m) => m.slug === params.section);
	const src = SOURCES[params.section];
	if (!meta || !src) error(404, 'manual section not found');
	const { html, toc } = renderMarkdown(parseFrontmatter(src).body.replace(/^\s*# .*\n/, ''));
	const ix = MANUAL_SECTIONS.findIndex((m) => m.slug === params.section);
	return {
		slug: params.section,
		title: meta.title,
		description: meta.description,
		html,
		toc,
		raw: parseFrontmatter(src).body,
		sections: splitRawSections(src),
		prev: ix > 0 ? MANUAL_SECTIONS[ix - 1] : null,
		next: ix < MANUAL_SECTIONS.length - 1 ? MANUAL_SECTIONS[ix + 1] : null
	};
}
