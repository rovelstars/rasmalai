import { error, redirect } from '@sveltejs/kit';
import { GUIDE_CHAPTERS } from '$lib/docs/nav';
import { parseFrontmatter, renderMarkdown, splitRawSections } from '$lib/docs/markdown';
const SOURCE_FILES = import.meta.glob('../../../content/guide/*.md', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

const SOURCES: Record<string, string> = Object.fromEntries(
	Object.entries(SOURCE_FILES).map(([file, raw]) => [file.split('/').pop()!.slice(0, -3), raw])
);
export function entries() {
	return GUIDE_CHAPTERS.map((c) => ({ chapter: c.slug }));
}

export async function load({ params }) {
	if (params.chapter === '09-ai-assistants') redirect(308, '/guide/10-ai-assistants');
	const meta = GUIDE_CHAPTERS.find((c) => c.slug === params.chapter);
	const src = SOURCES[params.chapter];
	if (!meta || !src) error(404, 'guide chapter not found');
	const { html, toc } = renderMarkdown(parseFrontmatter(src).body.replace(/^\s*# .*\n/, ''));
	const ix = GUIDE_CHAPTERS.findIndex((c) => c.slug === params.chapter);
	return {
		slug: params.chapter,
		title: meta.title,
		description: meta.description,
		html,
		toc,
		raw: parseFrontmatter(src).body,
		sections: splitRawSections(src),
		prev: ix > 0 ? GUIDE_CHAPTERS[ix - 1] : null,
		next: ix < GUIDE_CHAPTERS.length - 1 ? GUIDE_CHAPTERS[ix + 1] : null
	};
}
