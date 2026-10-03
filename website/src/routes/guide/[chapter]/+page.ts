import { error, redirect } from '@sveltejs/kit';
import { GUIDE_CHAPTERS } from '$lib/docs/nav';
import { parseFrontmatter, renderMarkdown, splitRawSections } from '$lib/docs/markdown';
import ch01 from '../../../content/guide/01-introduction.md?raw';
import ch02 from '../../../content/guide/02-basics-and-types.md?raw';
import ch03 from '../../../content/guide/03-control-flow.md?raw';
import ch04 from '../../../content/guide/04-functions-and-closures.md?raw';
import ch05 from '../../../content/guide/05-data-structures.md?raw';
import ch06 from '../../../content/guide/06-collections.md?raw';
import ch07 from '../../../content/guide/07-error-handling.md?raw';
import ch08 from '../../../content/guide/08-modules-and-packages.md?raw';
import ch09 from '../../../content/guide/09-editor-setup.md?raw';
import ch10 from '../../../content/guide/10-ai-assistants.md?raw';
import ch11 from '../../../content/guide/11-troubleshooting-and-reinstall.md?raw';

const SOURCES: Record<string, string> = {
	'01-introduction': ch01,
	'02-basics-and-types': ch02,
	'03-control-flow': ch03,
	'04-functions-and-closures': ch04,
	'05-data-structures': ch05,
	'06-collections': ch06,
	'07-error-handling': ch07,
	'08-modules-and-packages': ch08,
	'09-editor-setup': ch09,
	'10-ai-assistants': ch10,
	'11-troubleshooting-and-reinstall': ch11
};

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
