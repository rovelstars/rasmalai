// Builds social banners (1200x630) used for og:image / twitter:image.
// Runs in `prebuild` so PNGs regenerate on every deploy; outputs are
// gitignored. Rendering is pure SVG->PNG via @resvg/resvg-js: no browser,
// no system rasterizer. Brand type uses vendored Inter TTFs
// (static/fonts) so the banner matches the navbar exactly on any image.
//
// Three banner shapes: the home banner (og-banner.png, the default for
// every page without its own), doc sections (guide/manual/@std), and
// registry packages (name, version, downloads, description, best-effort
// from the live catalog API). og/manifest.json maps page keys to files;
// src/lib/docs/og.ts resolves a pathname through that manifest with a
// default-banner fallback, so a skipped variant can never 404 an embed.
import { writeFileSync, mkdirSync, readdirSync, readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Resvg } from '@resvg/resvg-js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, 'static', 'og');
mkdirSync(outDir, { recursive: true });

const SITE = (process.env['OG_SITE_URL'] ?? 'https://rasmalai.rovelstars.com').replace(/\/$/, '');

const W = 1200;
const H = 630;
const BG = '#14131a';
const PURPLE = '#a277ff';
const CYAN = '#67e8f9';
const TEXT = '#f4f2fa';
const MUTED = '#8b8798';
const FONT = 'Inter, sans-serif';

const fonts = {
	fontFiles: [join(root, 'static', 'fonts', 'Inter-SemiBold.ttf'), join(root, 'static', 'fonts', 'Inter-Regular.ttf')],
	loadSystemFonts: false
};

function esc(s) {
	return String(s ?? '')
		.replace(/&/g, '&amp;')
		.replace(/</g, '&lt;')
		.replace(/>/g, '&gt;')
		.replace(/"/g, '&quot;');
}

function headSize(text) {
	const len = [...text].length;
	if (len <= 24) return 96;
	if (len <= 40) return 74;
	return 58;
}

function trunc(text, n) {
	const t = String(text ?? '').replace(/\s+/g, ' ').trim();
	return t.length > n ? t.slice(0, n - 1).trimEnd() + '…' : t;
}

function shortDownloads(n) {
	if (!Number.isFinite(n) || n < 0) return '';
	if (n >= 1000000) return `${(n / 1000000).toFixed(1).replace(/\.0$/, '')}M`;
	if (n >= 1000) return `${(n / 1000).toFixed(1).replace(/\.0$/, '')}k`;
	return `${n}`;
}

function chrome(kicker, headline, subline, meta) {
	const size = headSize(headline);
	return (
		`<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">` +
		`<rect width="${W}" height="${H}" fill="${BG}"/>` +
		Array.from({ length: 13 }, (_, i) => `<line x1="${i * 100}" y1="0" x2="${i * 100}" y2="${H}" stroke="#ffffff" stroke-opacity="0.04"/>`).join('') +
		Array.from({ length: 8 }, (_, i) => `<line x1="0" y1="${i * 90}" x2="${W}" y2="${i * 90}" stroke="#ffffff" stroke-opacity="0.04"/>`).join('') +
		`<rect x="60" y="60" width="1080" height="510" rx="24" fill="none" stroke="${PURPLE}" stroke-opacity="0.45" stroke-width="2"/>` +
		`<circle cx="150" cy="150" r="90" fill="${PURPLE}" fill-opacity="0.14"/>` +
		`<circle cx="1070" cy="500" r="120" fill="${CYAN}" fill-opacity="0.08"/>` +
		`<text x="130" y="212" font-family="${FONT}" font-size="30" font-weight="600" letter-spacing="4" fill="${PURPLE}">${esc(kicker).toUpperCase()}</text>` +
		`<text x="126" y="${212 + size + 24}" font-family="${FONT}" font-size="${size}" font-weight="600" letter-spacing="-2" fill="${TEXT}">${esc(headline)}</text>` +
		`<text x="130" y="452" font-family="${FONT}" font-size="34" fill="${MUTED}">${esc(subline)}</text>` +
		(meta
			? `<text x="130" y="528" font-family="${FONT}" font-size="28" font-weight="600" fill="${CYAN}">${esc(meta)}</text>`
			: `<text x="130" y="528" font-family="${FONT}" font-size="28" fill="${PURPLE}">ARC memory · Cranelift JIT · LLVM releases</text>`) +
		`</svg>`
	);
}

function brand() {
	return (
		`<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">` +
		`<rect width="${W}" height="${H}" fill="${BG}"/>` +
		Array.from({ length: 13 }, (_, i) => `<line x1="${i * 100}" y1="0" x2="${i * 100}" y2="${H}" stroke="#ffffff" stroke-opacity="0.04"/>`).join('') +
		Array.from({ length: 8 }, (_, i) => `<line x1="0" y1="${i * 90}" x2="${W}" y2="${i * 90}" stroke="#ffffff" stroke-opacity="0.04"/>`).join('') +
		`<rect x="60" y="60" width="1080" height="510" rx="24" fill="none" stroke="${PURPLE}" stroke-opacity="0.45" stroke-width="2"/>` +
		`<circle cx="150" cy="150" r="90" fill="${PURPLE}" fill-opacity="0.14"/>` +
		`<circle cx="1070" cy="500" r="120" fill="${CYAN}" fill-opacity="0.08"/>` +
		`<text x="126" y="300" font-family="${FONT}" font-size="120" font-weight="600" letter-spacing="-3" fill="${TEXT}">Rasmalai</text>` +
		`<text x="130" y="372" font-family="${FONT}" font-size="38" fill="${MUTED}">Memory safety without the negotiations.</text>` +
		`<text x="130" y="480" font-family="${FONT}" font-size="30" fill="${PURPLE}">ARC memory · Cranelift JIT · LLVM releases</text>` +
		`</svg>`
	);
}

function renderTo(dir, file, svg) {
	const png = new Resvg(svg, { fitTo: { mode: 'width', value: W }, font: fonts }).render().asPng();
	writeFileSync(join(dir, file), png);
}

function render(svg, file) {
	renderTo(outDir, file, svg);
}

function frontmatter(path) {
	const text = readFileSync(path, 'utf8');
	if (!text.startsWith('---')) return {};
	const end = text.indexOf('\n---', 3);
	if (end < 0) return {};
	const out = {};
	for (const line of text.slice(3, end).split('\n')) {
		const m = /^([A-Za-z_]+):\s*"(.*)"\s*$/.exec(line.trim());
		if (m) out[m[1]] = m[2];
	}
	return out;
}

function slugFile(slug) {
	return slug.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'index';
}

const manifest = {};

renderTo(join(root, 'static'), 'og-banner.png', brand());
manifest['home'] = 'og-banner.png';

for (const [dir, kind] of [
	['guide', 'Guide'],
	['manual', 'Manual']
]) {
	let files = [];
	try {
		files = readdirSync(join(root, 'src', 'content', dir)).filter((f) => f.endsWith('.md'));
	} catch {
		console.warn(`og: no ${dir} content dir, skipping`);
	}
	for (const f of files) {
		const slug = f.replace(/\.md$/, '');
		const meta = frontmatter(join(root, 'src', 'content', dir, f));
		const file = `og-${kind.toLowerCase()}-${slugFile(slug)}.png`;
		render(
			chrome(kind, trunc(meta.title ?? slug, 48), trunc(meta.description ?? '', 130), null),
			file
		);
		manifest[`${dir}/${slug}`] = file;
	}
}

try {
	const mods = readdirSync(join(root, '..', 'compiler', 'stdlib', 'src'))
		.filter((f) => f.endsWith('.rnx'))
		.map((f) => f.replace(/\.rnx$/, ''))
		.filter((m) => !m.includes('/'));
	for (const m of mods) {
		const file = `og-std-${slugFile(m)}.png`;
		render(chrome('Standard Library', `@std/${m}`, 'Shipped with the compiler — nothing to install.', null), file);
		manifest[`std/${m}`] = file;
	}
} catch {
	console.warn('og: no compiler stdlib dir, skipping std banners');
}

try {
	const res = await fetch(`${SITE}/api/packages`);
	if (!res.ok) throw new Error(`catalog ${res.status}`);
	const body = await res.json();
	const pkgs = Array.isArray(body.packages) ? body.packages : [];
	for (const p of pkgs) {
		if (!p || typeof p.name !== 'string') continue;
		const file = `og-pkg-${slugFile(p.name.replace(/^@/, ''))}.png`;
		const stats = [p.latest ? `v${p.latest}` : null, shortDownloads(p.downloads) ? `${shortDownloads(p.downloads)} downloads` : null]
			.filter(Boolean)
			.join(' · ');
		render(
			chrome('Package', trunc(p.name, 40), trunc(p.description ?? '', 120), stats || null),
			file
		);
		manifest[`pkg/${p.name}`] = file;
	}
	console.log(`og: ${pkgs.length} package banners`);
} catch (e) {
	console.warn(`og: catalog unreachable (${e.message ?? e}), package banners skipped`);
}

writeFileSync(join(outDir, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n');
console.log(`og: ${Object.keys(manifest).length} entries`);
