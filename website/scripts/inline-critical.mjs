// Inlines above-the-fold CSS into every built HTML page so first paint
// is styled even when stylesheets are still in flight (Firefox paints
// before slow stylesheets arrive). Runs as `postbuild` after `vite build`.
// Content-based: no asset filenames are referenced anywhere.
import { readFileSync, writeFileSync, readdirSync, statSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import Critters from 'critters';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, '.svelte-kit', 'cloudflare');

function collect(dir, out = []) {
	for (const f of readdirSync(dir)) {
		const full = join(dir, f);
		if (statSync(full).isDirectory()) collect(full, out);
		else if (f.endsWith('.html')) out.push(full);
	}
	return out;
}

const critters = new Critters({
	path: outDir,
	publicPath: '/',
	logLevel: 'warn',
	inlineFonts: false,
	preload: 'swap',
	pruneSource: false,
	reduceInlineStyles: true
});

const files = collect(outDir);
let inlined = 0;
for (const file of files) {
	const html = readFileSync(file, 'utf8');
	if (!html.includes('rel="stylesheet"')) continue;
	try {
		const out = await critters.process(html);
		if (out !== html) {
			writeFileSync(file, out);
			inlined++;
		}
	} catch (e) {
		console.error(`inline-critical: skip ${file}: ${e.message}`);
	}
}
console.log(`inline-critical: inlined above-fold CSS in ${inlined}/${files.length} pages`);
