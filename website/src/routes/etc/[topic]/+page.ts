import { error } from '@sveltejs/kit';
import { ETC_SECTIONS } from '$lib/docs/nav';
import { parseFrontmatter, renderMarkdown, splitRawSections } from '$lib/docs/markdown';
const SOURCE_FILES = import.meta.glob('../../../content/etc/*.md', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

const SOURCES: Record<string, string> = Object.fromEntries(
	Object.entries(SOURCE_FILES).map(([file, raw]) => [file.split('/').pop()!.slice(0, -3), raw])
);
export function entries() {
	return ETC_SECTIONS.map((m) => ({ topic: m.slug }));
}

export async function load({ params }) {
	const meta = ETC_SECTIONS.find((m) => m.slug === params.topic);
	const src = SOURCES[params.topic];
	if (!meta || !src) error(404, 'etc topic not found');
	const { html, toc } = renderMarkdown(parseFrontmatter(src).body.replace(/^\s*# .*\n/, ''));
	const ix = ETC_SECTIONS.findIndex((m) => m.slug === params.topic);
	return {
		slug: params.topic,
		title: meta.title,
		description: meta.description,
		html,
		toc,
		raw: parseFrontmatter(src).body,
		sections: splitRawSections(src),
		prev: ix > 0 ? ETC_SECTIONS[ix - 1] : null,
		next: ix < ETC_SECTIONS.length - 1 ? ETC_SECTIONS[ix + 1] : null
	};
}
