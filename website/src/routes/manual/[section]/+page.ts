import { error, redirect } from '@sveltejs/kit';
import { MANUAL_SECTIONS, LEGACY_MANUAL_REDIRECTS } from '$lib/docs/nav';
import { parseFrontmatter, renderMarkdown, splitRawSections } from '$lib/docs/markdown';
import m01 from '../../../content/manual/01-lexicon-and-structure.md?raw';
import m01a from '../../../content/manual/01a-prelude-and-intrinsics.md?raw';
import m02 from '../../../content/manual/02-numeric-model.md?raw';
import m03 from '../../../content/manual/03-bindings-and-scope.md?raw';
import m04 from '../../../content/manual/04-control-flow.md?raw';
import m05 from '../../../content/manual/05-functions-and-closures.md?raw';
import m06 from '../../../content/manual/06-structs-and-records.md?raw';
import m07 from '../../../content/manual/07-classes-and-objects.md?raw';
import m08 from '../../../content/manual/08-traits-and-interfaces.md?raw';
import m09 from '../../../content/manual/09-extensions-and-operators.md?raw';
import m10 from '../../../content/manual/10-enums-and-matching.md?raw';
import m11 from '../../../content/manual/11-memory-and-arc.md?raw';
import m12 from '../../../content/manual/12-cycles-and-handles.md?raw';
import m13 from '../../../content/manual/13-hardware-and-ffi.md?raw';
import m14 from '../../../content/manual/14-concurrency-and-threads.md?raw';
import m15 from '../../../content/manual/15-vectorization-and-simd.md?raw';
import m16 from '../../../content/manual/16-project-and-toolchain.md?raw';
import m17 from '../../../content/manual/17-diagnostics-directory.md?raw';

const SOURCES: Record<string, string> = {
	'01-lexicon-and-structure': m01,
	'01a-prelude-and-intrinsics': m01a,
	'02-numeric-model': m02,
	'03-bindings-and-scope': m03,
	'04-control-flow': m04,
	'05-functions-and-closures': m05,
	'06-structs-and-records': m06,
	'07-classes-and-objects': m07,
	'08-traits-and-interfaces': m08,
	'09-extensions-and-operators': m09,
	'10-enums-and-matching': m10,
	'11-memory-and-arc': m11,
	'12-cycles-and-handles': m12,
	'13-hardware-and-ffi': m13,
	'14-concurrency-and-threads': m14,
	'15-vectorization-and-simd': m15,
	'16-project-and-toolchain': m16,
	'17-diagnostics-directory': m17
};

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
