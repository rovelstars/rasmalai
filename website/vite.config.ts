import adapter from '@sveltejs/adapter-cloudflare';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig, type Plugin } from 'vite';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';

// workerd refuses to compile Wasm from bytes at runtime, so the server
// analyzer must reach the deploy bundler as a static `*.wasm?module`
// import: wrangler uploads it as a precompiled module and the default
// export is a WebAssembly.Module. Vite would instead emit a fetch plus
// runtime compile, which the embedder forbids, so the import is
// externalized here and the bytes are emitted beside the importing chunk
// for wrangler to resolve at deploy time. A bare `*.wasm` import without
// the suffix must keep failing the build loudly: it would mean this
// plugin no longer guards the server path.
function compiledWasm(): Plugin {
	const sources = new Map<string, { abs: string; bytes: Uint8Array }>();
	return {
		name: 'rasmalai-compiled-wasm',
		enforce: 'pre',
		resolveId(source, importer) {
			if (!source.endsWith('.wasm?module')) return null;
			if (this.environment?.config?.consumer !== 'server') return null;
			if (!importer) throw new Error(`[rasmalai-compiled-wasm] ${source} has no importer`);
			const abs = path.resolve(path.dirname(importer.split('?')[0]), source.split('?')[0]);
			if (!existsSync(abs)) throw new Error(`[rasmalai-compiled-wasm] ${abs} does not exist`);
			const id = `./${path.basename(abs)}?module`;
			const known = sources.get(id);
			if (known && known.abs !== abs)
				throw new Error(`[rasmalai-compiled-wasm] ${known.abs} and ${abs} share the import name ${id}`);
			sources.set(id, { abs, bytes: new Uint8Array(readFileSync(abs)) });
			return { id, external: true };
		},
		generateBundle(options, bundle) {
			if (sources.size === 0) return;
			if (!options.dir) throw new Error('[rasmalai-compiled-wasm] no output directory');
			for (const chunk of Object.values(bundle)) {
				if (chunk.type !== 'chunk') continue;
				for (const id of chunk.imports) {
					const found = sources.get(id);
					if (!found) continue;
					this.emitFile({
						type: 'asset',
						fileName: path.posix.join(path.posix.dirname(chunk.fileName), path.basename(found.abs)),
						source: found.bytes
					});
				}
			}
		}
	};
}

export default defineConfig({
	plugins: [
		compiledWasm(),
		sveltekit({
			compilerOptions: {
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},
			adapter: adapter(),
			prerender: {
				// Playground share links carry snippet hashes unknown at build time.
				handleMissingId: 'ignore'
			}
		})
	]
});
