import {
	GUIDE_CHAPTERS,
	MANUAL_SECTIONS,
	ROSETTA,
	TRACKS,
	STDLIB
} from '$lib/docs/nav';

const BASE = 'https://rnx.dev';

const STATIC_PAGES = ['/', '/guide', '/manual', '/packages', '/playground', '/license', '/discord'];

export async function GET() {
	const urls: string[] = [...STATIC_PAGES];
	for (const c of GUIDE_CHAPTERS) urls.push(c.path);
	for (const m of MANUAL_SECTIONS) urls.push(m.path);
	for (const r of ROSETTA) urls.push(r.path);
	for (const t of TRACKS) urls.push(`/guide/tracks/${t.slug}`);
	for (const m of STDLIB) {
		urls.push(`/packages/@std/${m}`);
		urls.push(`/docs/@std/${m}/overview`);
		urls.push(`/docs/@std/${m}/getting-started`);
		urls.push(`/docs/@std/${m}/api`);
	}
	const body =
		`<?xml version="1.0" encoding="UTF-8"?>\n` +
		`<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n` +
		urls.map((u) => `  <url><loc>${BASE}${u}</loc></url>`).join('\n') +
		`\n</urlset>\n`;
	return new Response(body, {
		headers: { 'content-type': 'application/xml; charset=utf-8' }
	});
}
