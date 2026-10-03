// Builds the social banner (1200x630) used for og:image / twitter:image.
// Runs in `prebuild` so the PNG is generated on every deploy; the output
// is gitignored. Rendering is pure SVG->PNG via @resvg/resvg-js: no
// browser, no system rasterizer, works on any CI image.
import { writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Resvg } from '@resvg/resvg-js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, 'static');
mkdirSync(outDir, { recursive: true });

const W = 1200;
const H = 630;
const BG = '#14131a';
const PURPLE = '#a277ff';
const CYAN = '#67e8f9';
const TEXT = '#f4f2fa';
const MUTED = '#8b8798';

const svg =
	`<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">` +
	`<rect width="${W}" height="${H}" fill="${BG}"/>` +
	Array.from({ length: 13 }, (_, i) => `<line x1="${i * 100}" y1="0" x2="${i * 100}" y2="${H}" stroke="#ffffff" stroke-opacity="0.04"/>`).join('') +
	Array.from({ length: 8 }, (_, i) => `<line x1="0" y1="${i * 90}" x2="${W}" y2="${i * 90}" stroke="#ffffff" stroke-opacity="0.04"/>`).join('') +
	`<rect x="60" y="60" width="1080" height="510" rx="24" fill="none" stroke="${PURPLE}" stroke-opacity="0.45" stroke-width="2"/>` +
	`<circle cx="150" cy="150" r="90" fill="${PURPLE}" fill-opacity="0.14"/>` +
	`<circle cx="1070" cy="500" r="120" fill="${CYAN}" fill-opacity="0.08"/>` +
	`<text x="130" y="270" font-family="DejaVu Sans, Verdana, sans-serif" font-size="120" font-weight="bold" fill="${TEXT}" letter-spacing="-3">Rasmalai</text>` +
	`<text x="134" y="340" font-family="DejaVu Sans, Verdana, sans-serif" font-size="38" fill="${MUTED}">Memory safety without the negotiations.</text>` +
	`<rect x="130" y="400" width="560" height="76" rx="38" fill="#1e1c28" stroke="#2e2b3d" stroke-width="2"/>` +
	`<text x="166" y="449" font-family="DejaVu Sans Mono, monospace" font-size="30" fill="${CYAN}">$ curl -fsSL rnx.dev/install.sh | sh</text>` +
	`<text x="134" y="540" font-family="DejaVu Sans, Verdana, sans-serif" font-size="26" fill="${PURPLE}">ARC memory · Cranelift JIT · LLVM releases</text>` +
	`</svg>`;

const resvg = new Resvg(svg, {
	fitTo: { mode: 'width', value: W },
	font: { loadSystemFonts: true }
});
writeFileSync(join(outDir, 'og-banner.png'), resvg.render().asPng());
console.log('og banner written');
